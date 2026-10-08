use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::OffsetDateTime;

pub const MAX_SLUG_LENGTH: usize = 100;
pub const SETTLEMENT_URL_PREFIX: &str = "/naselennyy-punkt/";

/// JavaScript string length in UTF-16 code units, used for every bound that
/// the TypeScript stores compared with `value.length`.
pub fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// The exact JavaScript `\s` set (ECMAScript WhiteSpace + LineTerminator),
/// which `String.prototype.trim` and `/\s+/gu` operate on.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | ' '
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

pub fn js_trim(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

/// `value.trim().replace(/\s+/gu, " ")` from the settings store.
pub fn js_collapse_whitespace(value: &str) -> String {
    js_trim(value)
        .split(is_js_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `Number(value)` semantics for finite checks: invalid text and NaN inputs
/// yield NaN, hex/binary/octal literals parse like JavaScript, and the callers
/// keep their `Number.isFinite` checks.
pub fn js_number(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16u32), ("0X", 16), ("0b", 2), ("0B", 2), ("0o", 8), ("0O", 8)] {
        if let Some(rest) = value.strip_prefix(prefix) {
            if rest.is_empty() {
                return f64::NAN;
            }
            return match i128::from_str_radix(rest, radix) {
                Ok(parsed) => parsed as f64,
                Err(_) => f64::NAN,
            };
        }
    }
    value.parse::<f64>().unwrap_or(f64::NAN)
}

/// `new Date().toISOString()` shape: millisecond precision, UTC, `Z` suffix.
pub fn iso_now() -> String {
    let format = format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    OffsetDateTime::now_utc()
        .format(&format)
        .unwrap_or_else(|_| "1970-01-01T00:00:00.000Z".to_string())
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// `Date.parse` for the stored timestamps. Accepted shapes cover the real data
/// (RFC 3339) plus the common zone-less variants; anything else is rejected the
/// same way the stores reject non-finite `Date.parse` results.
pub fn parse_js_date_ms(value: &str) -> Option<i64> {
    if let Ok(parsed) = OffsetDateTime::parse(value, &Rfc3339) {
        return Some(millis(&parsed));
    }
    let formats = [
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]"),
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]"),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]"),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"),
    ];
    for format in formats {
        if let Ok(parsed) = time::PrimitiveDateTime::parse(value, &format) {
            return Some(millis(&parsed.assume_utc()));
        }
    }
    None
}

fn millis(value: &OffsetDateTime) -> i64 {
    (value.unix_timestamp_nanos() / 1_000_000) as i64
}

pub fn is_valid_slug(value: &str) -> bool {
    if value.is_empty() || utf16_len(value) > MAX_SLUG_LENGTH {
        return false;
    }
    value.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

pub fn is_valid_settlement_url(value: &str) -> bool {
    value
        .strip_prefix(SETTLEMENT_URL_PREFIX)
        .is_some_and(is_valid_slug)
}

/// Stored settlements may omit their page address; the legacy derivation keeps
/// every settlement reachable either way.
pub fn effective_settlement_url(url: Option<&str>, settlement_id: &str) -> String {
    match url {
        Some(url) => url.to_string(),
        None => format!("{SETTLEMENT_URL_PREFIX}{settlement_id}"),
    }
}

pub fn random_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Russian collation approximation of `new Intl.Collator("ru")` for the
/// province names stored in the atlas: letters compare by the Russian alphabet
/// (ё after е), spaces are primary-ignorable, other characters fall back to
/// code points, and equal primaries fall back to a plain code point compare.
const RU_ALPHABET: &str = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя";

fn ru_weight(c: char) -> i32 {
    if let Some(index) = RU_ALPHABET.chars().position(|letter| letter == c) {
        return index as i32;
    }
    if c == ' ' || c == '\u{00A0}' {
        return -1;
    }
    1_000 + c as i32
}

pub fn ru_compare(left: &str, right: &str) -> Ordering {
    let left_weights: Vec<i32> = left.to_lowercase().chars().map(ru_weight).collect();
    let right_weights: Vec<i32> = right.to_lowercase().chars().map(ru_weight).collect();
    match left_weights.cmp(&right_weights) {
        Ordering::Equal => left.cmp(right),
        ordering => ordering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_len_counts_code_units() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("🗺"), 2);
        assert_eq!(utf16_len("ё"), 1);
    }

    #[test]
    fn js_number_matches_javascript_shapes() {
        assert_eq!(js_number("12.5"), 12.5);
        assert_eq!(js_number("-3"), -3.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert_eq!(js_number("0b101"), 5.0);
        assert_eq!(js_number(""), 0.0);
        assert!(js_number("nope").is_nan());
        assert!(js_number("0x").is_nan());
        assert!(!js_number("Infinity").is_finite());
    }

    #[test]
    fn slug_validation_matches_store_pattern() {
        assert!(is_valid_slug("ryazanskaya"));
        assert!(is_valid_slug("gubernia-1897-2"));
        assert!(!is_valid_slug("Rязань"));
        assert!(!is_valid_slug("-a"));
        assert!(!is_valid_slug("a-"));
        assert!(!is_valid_slug("a--b"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug(&"a".repeat(101)));
    }

    #[test]
    fn settlement_urls_validate_with_prefix() {
        assert!(is_valid_settlement_url("/naselennyy-punkt/pavelets"));
        assert!(is_valid_settlement_url(
            "/naselennyy-punkt/dbd08506-dde4-4a9b-b644-271d33e8b2e2"
        ));
        assert!(!is_valid_settlement_url("/naselennyy-punkt/"));
        assert!(!is_valid_settlement_url("pavelets"));
    }

    #[test]
    fn collation_matches_icu_fixture() {
        let fixture = include_str!("../tests/fixtures/collation_expected.json");
        let expected: Vec<String> = serde_json::from_str(fixture).expect("fixture parses");
        let mut names = expected.clone();
        names.sort_by(|left, right| ru_compare(left, right));
        assert_eq!(names, expected);
    }

    #[test]
    fn date_parsing_handles_stored_shapes() {
        assert_eq!(
            parse_js_date_ms("2026-10-07T13:54:44.905Z"),
            Some(1_791_381_284_905)
        );
        assert!(parse_js_date_ms("not a date").is_none());
        assert!(parse_js_date_ms("2026-10-07T13:54:44").is_some());
    }
}
