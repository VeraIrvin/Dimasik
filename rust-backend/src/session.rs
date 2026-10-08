use axum::http::{header, HeaderMap};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::util::utf16_len;

pub const ADMIN_COOKIE: &str = "dimasik_admin_session";
pub const SESSION_MAX_AGE: i64 = 60 * 60 * 8;

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub username: Option<String>,
    pub password: Option<String>,
    pub secret: Option<String>,
}

fn sha256(value: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher.finalize().into()
}

fn sign(secret: &str, payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(payload.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

impl AuthConfig {
    /// Constant-time comparison of the sha256 digests, mirroring
    /// `checkAdminCredentials`; a missing configuration never authenticates.
    pub fn check_credentials(&self, username: &str, password: &str) -> bool {
        let (Some(configured_username), Some(configured_password)) =
            (self.username.as_deref(), self.password.as_deref())
        else {
            return false;
        };
        let username_matches: bool = sha256(username)
            .ct_eq(&sha256(configured_username))
            .into();
        let password_matches: bool = sha256(password)
            .ct_eq(&sha256(configured_password))
            .into();
        username_matches && password_matches
    }

    /// `signingSecret()`: secrets shorter than 32 characters are unusable.
    fn signing_secret(&self) -> Option<&str> {
        let secret = self.secret.as_deref()?;
        if utf16_len(secret) < 32 {
            return None;
        }
        Some(secret)
    }

    /// `createAdminSession()`: None when the secret is missing or too short,
    /// which the login route reports as the 503 "temporarily unavailable".
    pub fn create_session(&self, now: i64) -> Option<String> {
        let secret = self.signing_secret()?;
        let expires = now + SESSION_MAX_AGE;
        let payload = format!("admin.{expires}");
        let signature = sign(secret, &payload);
        Some(format!("{payload}.{signature}"))
    }

    pub fn has_admin_session(&self, headers: &HeaderMap, now: i64) -> bool {
        let Some(value) = cookie_value(headers, ADMIN_COOKIE) else {
            return false;
        };
        verify_session(self.signing_secret(), &value, now)
    }
}

/// `verifyAdminSession`: three dot-separated parts, `admin` prefix, numeric
/// expiry strictly inside the eight-hour window, and a constant-time HMAC
/// comparison against a freshly computed signature.
pub fn verify_session(secret: Option<&str>, value: &str, now: i64) -> bool {
    let Some(secret) = secret else {
        return false;
    };
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 || parts[0] != "admin" {
        return false;
    }
    if parts[1].is_empty() || !parts[1].bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Ok(expires) = parts[1].parse::<i64>() else {
        return false;
    };
    if expires <= now || expires > now + SESSION_MAX_AGE {
        return false;
    }
    let expected = sign(secret, &format!("{}.{}", parts[0], parts[1]));
    let Ok(expected_bytes) = URL_SAFE_NO_PAD.decode(expected.as_bytes()) else {
        return false;
    };
    let Ok(actual_bytes) = URL_SAFE_NO_PAD.decode(parts[2].as_bytes()) else {
        return false;
    };
    expected_bytes.len() == actual_bytes.len() && expected_bytes.ct_eq(&actual_bytes).into()
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(header::COOKIE) {
        let Ok(raw) = header.to_str() else {
            continue;
        };
        for pair in raw.split(';') {
            let pair = pair.trim();
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

pub fn set_cookie_header(token: &str, secure: bool) -> String {
    let mut value = format!(
        "{ADMIN_COOKIE}={token}; Max-Age={SESSION_MAX_AGE}; Path=/; HttpOnly; SameSite=Strict"
    );
    if secure {
        value.push_str("; Secure");
    }
    value
}

pub fn clear_cookie_header(secure: bool) -> String {
    let mut value = format!("{ADMIN_COOKIE}=; Max-Age=0; Path=/; HttpOnly; SameSite=Strict");
    if secure {
        value.push_str("; Secure");
    }
    value
}

/// Behind the Next.js proxy the browser scheme arrives as `X-Forwarded-Proto`;
/// local HTTP development leaves the cookie non-Secure.
pub fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("https")
        })
        .unwrap_or(false)
}

fn normalize_origin(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn js_url_host(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    let host = url.host_str()?;
    match url.port() {
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host.to_string()),
    }
}

/// `assertAdminMutation`'s cross-origin defence: a missing Origin passes (the
/// service-to-service SSR calls and curl), the configured frontend origin
/// always passes, and otherwise the Origin host must match the request host.
pub fn origin_allowed(headers: &HeaderMap, frontend_origin: Option<&str>) -> bool {
    let Some(origin_header) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin_header.to_str() else {
        return false;
    };
    if let Some(configured) = frontend_origin {
        if normalize_origin(origin) == normalize_origin(configured) {
            return true;
        }
    }
    let forwarded_host = headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_string());
    let request_host = forwarded_host.or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string())
    });
    let Some(request_host) = request_host else {
        return false;
    };
    js_url_host(origin).is_some_and(|origin_host| origin_host == request_host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn auth() -> AuthConfig {
        AuthConfig {
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            secret: Some("0123456789abcdef0123456789abcdef".to_string()),
        }
    }

    #[test]
    fn credentials_require_exact_match() {
        let auth = auth();
        assert!(auth.check_credentials("admin", "secret"));
        assert!(!auth.check_credentials("admin", "Secret"));
        assert!(!auth.check_credentials("root", "secret"));
        let empty = AuthConfig::default();
        assert!(!empty.check_credentials("admin", "secret"));
    }

    #[test]
    fn sessions_round_trip_and_expire() {
        let config = auth();
        let now = 1_000_000;
        let token = config.create_session(now).unwrap();
        assert!(verify_session(config.secret.as_deref(), &token, now));
        assert!(!verify_session(config.secret.as_deref(), &token, now - 1));
        assert!(!verify_session(
            config.secret.as_deref(),
            &token,
            now + SESSION_MAX_AGE + 1
        ));
        assert!(!verify_session(config.secret.as_deref(), "admin.1.deadbeef", now));
        assert!(!verify_session(None, &token, now));

        let short = AuthConfig {
            secret: Some("short".to_string()),
            ..config
        };
        assert!(short.create_session(now).is_none());
    }

    #[test]
    fn cookie_parsing_and_origin_rules() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; dimasik_admin_session=token.value; b=2"),
        );
        assert_eq!(
            cookie_value(&headers, ADMIN_COOKIE).as_deref(),
            Some("token.value")
        );

        assert!(origin_allowed(&HeaderMap::new(), Some("http://localhost:3000")));
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("http://localhost:3000"));
        assert!(origin_allowed(&headers, Some("http://localhost:3000")));
        assert!(origin_allowed(&headers, None) == false);
        assert!(!origin_allowed(&headers, Some("https://other.example")));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));
        assert!(origin_allowed(&headers, None));
    }
}
