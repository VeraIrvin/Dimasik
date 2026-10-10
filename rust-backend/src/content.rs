use serde_json::{json, Map, Value};
use url::Url;

use crate::util::{js_trim, utf16_len};

pub const MAX_POST_DOCUMENT_JSON_CHARACTERS: usize = 200_000;
const MAX_DOCUMENT_DEPTH: usize = 20;
const MAX_DOCUMENT_NODES: usize = 5_000;
const MAX_DOCUMENT_TEXT: usize = 100_000;
const MAX_LINK_HREF_LENGTH: usize = 2_048;
const MAX_LINK_TITLE_LENGTH: usize = 512;
const MAX_TABLE_ROWS: usize = 100;
const MAX_TABLE_COLUMNS: usize = 100;
const MAX_TABLE_COLUMN_WIDTH: usize = 10_000;

pub(crate) const EMPTY_DOCUMENT_MESSAGE: &str = "Текст публикации не должен быть пустым.";
const INVALID_DOCUMENT_MESSAGE: &str = "Некорректное содержимое публикации.";

#[derive(Debug)]
pub struct ContentError(pub &'static str);

type CResult<T> = Result<T, ContentError>;

#[derive(Default)]
struct JsonCharacterCount(usize);

impl std::io::Write for JsonCharacterCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        // JSON is UTF-8. Count leading bytes, with astral code points counting
        // twice; continuation bytes count zero even across writer chunks.
        for byte in bytes {
            self.0 += match *byte {
                0x00..=0x7f | 0xc0..=0xef => 1,
                0xf0..=0xff => 2,
                _ => 0,
            };
            if self.0 > MAX_POST_DOCUMENT_JSON_CHARACTERS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    INVALID_DOCUMENT_MESSAGE,
                ));
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn validate_document_json_size(value: &Value) -> CResult<()> {
    serde_json::to_writer(JsonCharacterCount::default(), value)
        .map_err(|_| ContentError(INVALID_DOCUMENT_MESSAGE))
}

fn fail<T>() -> CResult<T> {
    Err(ContentError(INVALID_DOCUMENT_MESSAGE))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parent {
    Doc,
    Blockquote,
    BulletList,
    OrderedList,
    ListItem,
    Inline,
    Table,
    TableRow,
    TableCell,
}

struct Context {
    nodes: usize,
    text_length: usize,
    has_visible_content: bool,
}

fn has_only_keys(record: &Map<String, Value>, allowed: &[&str]) -> bool {
    record.keys().all(|key| allowed.contains(&key.as_str()))
}

fn as_record(value: &Value) -> CResult<&Map<String, Value>> {
    match value {
        Value::Object(record) => Ok(record),
        _ => fail(),
    }
}

fn contains_control(value: &str) -> bool {
    value
        .chars()
        .any(|c| c <= '\u{001F}' || c == '\u{007F}')
}

fn node_keys(node_type: &str) -> Option<&'static [&'static str]> {
    Some(match node_type {
        "doc" => &["type", "content"],
        "paragraph" => &["type", "attrs", "content"],
        "heading" => &["type", "attrs", "content"],
        "blockquote" => &["type", "content"],
        "bulletList" => &["type", "content"],
        "orderedList" => &["type", "attrs", "content"],
        "listItem" => &["type", "content"],
        "table" | "tableRow" => &["type", "content"],
        "tableCell" | "tableHeader" => &["type", "attrs", "content"],
        "text" => &["type", "text", "marks"],
        "hardBreak" => &["type"],
        _ => return None,
    })
}

fn allowed_parent(node_type: &str, parent: Parent) -> bool {
    match node_type {
        "paragraph" | "heading" | "blockquote" | "bulletList" | "orderedList" | "table" => matches!(
            parent,
            Parent::Doc | Parent::Blockquote | Parent::ListItem | Parent::TableCell
        ),
        "listItem" => matches!(parent, Parent::BulletList | Parent::OrderedList),
        "tableRow" => matches!(parent, Parent::Table),
        "tableCell" | "tableHeader" => matches!(parent, Parent::TableRow),
        "text" | "hardBreak" => matches!(parent, Parent::Inline),
        _ => false,
    }
}

fn hex_color(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix('#')?;
    if rest.len() != 3 && rest.len() != 6 {
        return None;
    }
    if !rest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", rest.to_ascii_lowercase()))
}

fn rgb_color(raw: &str) -> Option<String> {
    if raw.len() < 5 {
        return None;
    }
    let prefix = raw.get(..4)?;
    if !prefix.eq_ignore_ascii_case("rgb(") || !raw.ends_with(')') {
        return None;
    }
    let inner = raw.get(4..raw.len() - 1)?;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut channels = [0u8; 3];
    for (index, part) in parts.iter().enumerate() {
        let trimmed = js_trim(part);
        if trimmed.is_empty()
            || trimmed.len() > 3
            || !trimmed.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let channel: u32 = trimmed.parse().ok()?;
        if channel > 255 {
            return None;
        }
        channels[index] = channel as u8;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channels[0], channels[1], channels[2]
    ))
}

fn normalize_color(value: &Value) -> CResult<String> {
    let Some(raw) = value.as_str() else {
        return fail();
    };
    if let Some(color) = hex_color(raw) {
        return Ok(color);
    }
    if let Some(color) = rgb_color(raw) {
        return Ok(color);
    }
    fail()
}

fn normalize_link_href(value: &Value) -> CResult<String> {
    let Some(raw) = value.as_str() else {
        return fail();
    };
    if raw.is_empty() || utf16_len(raw) > MAX_LINK_HREF_LENGTH {
        return fail();
    }
    if contains_control(raw) || raw.contains('\\') {
        return fail();
    }
    if raw.starts_with('/') {
        // Root-relative paths only: "//host" is protocol-relative, not a path.
        if raw.starts_with("//") {
            return fail();
        }
        return Ok(raw.to_string());
    }
    match Url::parse(raw) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => Ok(raw.to_string()),
        _ => fail(),
    }
}

fn normalize_link_attrs(attrs: &Map<String, Value>) -> CResult<Value> {
    if !has_only_keys(attrs, &["href", "target", "rel", "class", "title"]) {
        return fail();
    }
    if let Some(target) = attrs.get("target") {
        if !target.is_null() && target.as_str() != Some("_blank") {
            return fail();
        }
    }
    if let Some(rel) = attrs.get("rel") {
        if !rel.is_null() && rel.as_str() != Some("noopener noreferrer nofollow") {
            return fail();
        }
    }
    if let Some(class_name) = attrs.get("class") {
        if !class_name.is_null() {
            return fail();
        }
    }

    let href = normalize_link_href(attrs.get("href").unwrap_or(&Value::Null))?;
    let mut out = Map::new();
    out.insert("href".to_string(), Value::String(href));
    match attrs.get("title") {
        None | Some(Value::Null) => {}
        Some(Value::String(title)) => {
            if title.is_empty()
                || utf16_len(title) > MAX_LINK_TITLE_LENGTH
                || contains_control(title)
            {
                return fail();
            }
            out.insert("title".to_string(), Value::String(title.clone()));
        }
        Some(_) => return fail(),
    }
    Ok(Value::Object(out))
}

fn normalize_marks(value: Option<&Value>) -> CResult<Option<Value>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail();
    };
    if items.len() > 8 {
        return fail();
    }
    if items.is_empty() {
        return Ok(None);
    }

    let mut marks: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for candidate in items {
        let Some(mark) = normalize_mark(candidate)? else {
            continue;
        };
        let mark_type = mark
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if seen.contains(&mark_type) {
            return fail();
        }
        seen.push(mark_type);
        marks.push(mark);
    }
    if marks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Value::Array(marks)))
}

fn normalize_mark(value: &Value) -> CResult<Option<Value>> {
    let mark = as_record(value)?;
    if !has_only_keys(mark, &["type", "attrs"]) {
        return fail();
    }
    let Some(mark_type) = mark.get("type").and_then(Value::as_str) else {
        return fail();
    };

    match mark_type {
        "bold" | "italic" | "strike" | "underline" => {
            if let Some(attrs) = mark.get("attrs") {
                if !attrs.is_null() {
                    let attrs = as_record(attrs)?;
                    if !attrs.is_empty() {
                        return fail();
                    }
                }
            }
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String(mark_type.to_string()));
            Ok(Some(Value::Object(out)))
        }
        "textStyle" => {
            let attrs = as_record(mark.get("attrs").unwrap_or(&Value::Null))?;
            if !has_only_keys(attrs, &["color"]) {
                return fail();
            }
            match attrs.get("color") {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(color)) if color.is_empty() => Ok(None),
                Some(color) => {
                    let color = normalize_color(color)?;
                    let mut attrs_out = Map::new();
                    attrs_out.insert("color".to_string(), Value::String(color));
                    let mut out = Map::new();
                    out.insert("type".to_string(), Value::String("textStyle".to_string()));
                    out.insert("attrs".to_string(), Value::Object(attrs_out));
                    Ok(Some(Value::Object(out)))
                }
            }
        }
        "link" => {
            let attrs = as_record(mark.get("attrs").unwrap_or(&Value::Null))?;
            let attrs = normalize_link_attrs(attrs)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("link".to_string()));
            out.insert("attrs".to_string(), attrs);
            Ok(Some(Value::Object(out)))
        }
        _ => fail(),
    }
}

fn normalize_alignment_attrs(value: Option<&Value>) -> CResult<Option<Value>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let attrs = as_record(value)?;
    if !has_only_keys(attrs, &["textAlign"]) {
        return fail();
    }
    match attrs.get("textAlign") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(alignment))
            if matches!(alignment.as_str(), "left" | "center" | "right" | "justify") =>
        {
            let mut out = Map::new();
            out.insert(
                "textAlign".to_string(),
                Value::String(alignment.clone()),
            );
            Ok(Some(Value::Object(out)))
        }
        _ => fail(),
    }
}

fn normalize_heading_attrs(value: Option<&Value>) -> CResult<Value> {
    let attrs = as_record(value.unwrap_or(&Value::Null))?;
    if !has_only_keys(attrs, &["level", "textAlign"]) {
        return fail();
    }
    let level = match attrs.get("level").and_then(Value::as_f64) {
        Some(level) if level == 2.0 || level == 3.0 => level as i64,
        _ => return fail(),
    };

    let mut out = Map::new();
    out.insert("level".to_string(), Value::from(level));
    match attrs.get("textAlign") {
        None | Some(Value::Null) => {}
        Some(Value::String(alignment))
            if matches!(alignment.as_str(), "left" | "center" | "right" | "justify") =>
        {
            out.insert(
                "textAlign".to_string(),
                Value::String(alignment.clone()),
            );
        }
        _ => return fail(),
    }
    Ok(Value::Object(out))
}

fn normalize_ordered_list_attrs(value: Option<&Value>) -> CResult<Option<Value>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let attrs = as_record(value)?;
    if !has_only_keys(attrs, &["start"]) {
        return fail();
    }
    match attrs.get("start") {
        None | Some(Value::Null) => Ok(None),
        Some(start) if start.as_f64() == Some(1.0) => Ok(None),
        Some(start) => {
            // `Number.isSafeInteger` parity: JSON integral floats such as 2.0
            // or 1e6 are valid and canonicalise to plain integers.
            let number = match start.as_f64() {
                Some(number)
                    if number.is_finite()
                        && number.fract() == 0.0
                        && (1.0..=1_000_000.0).contains(&number)
                        && number <= 9_007_199_254_740_991.0 =>
                {
                    number as i64
                }
                _ => return fail(),
            };
            let mut out = Map::new();
            out.insert("start".to_string(), Value::from(number));
            Ok(Some(Value::Object(out)))
        }
    }
}

fn bounded_integer(value: &Value, minimum: usize, maximum: usize) -> CResult<usize> {
    match value.as_f64() {
        Some(number)
            if number.is_finite()
                && number.fract() == 0.0
                && (minimum as f64..=maximum as f64).contains(&number) =>
        {
            Ok(number as usize)
        }
        _ => fail(),
    }
}

fn normalize_table_cell_attrs(value: Option<&Value>) -> CResult<Value> {
    let attrs = value.map(as_record).transpose()?;
    if attrs.is_some_and(|attrs| {
        !has_only_keys(attrs, &["colspan", "rowspan", "colwidth", "align"])
    }) {
        return fail();
    }
    let get = |key: &str| attrs.and_then(|attrs| attrs.get(key));
    let colspan = get("colspan")
        .map(|value| bounded_integer(value, 1, MAX_TABLE_COLUMNS))
        .transpose()?
        .unwrap_or(1);
    let rowspan = get("rowspan")
        .map(|value| bounded_integer(value, 1, MAX_TABLE_ROWS))
        .transpose()?
        .unwrap_or(1);
    let colwidth = match get("colwidth") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Array(widths)) if widths.len() == colspan => {
            let mut normalized = Vec::with_capacity(widths.len());
            for width in widths {
                normalized.push(Value::from(bounded_integer(width, 0, MAX_TABLE_COLUMN_WIDTH)?));
            }
            Value::Array(normalized)
        }
        _ => return fail(),
    };
    let align = match get("align") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(align)) if matches!(align.as_str(), "left" | "center" | "right") => {
            Value::String(align.clone())
        }
        _ => return fail(),
    };
    Ok(json!({
        "colspan": colspan,
        "rowspan": rowspan,
        "colwidth": colwidth,
        "align": align,
    }))
}

fn normalize_table_children(
    node: &Map<String, Value>,
    parent: Parent,
    depth: usize,
    context: &mut Context,
) -> CResult<Value> {
    let Some(Value::Array(items)) = node.get("content") else {
        return fail();
    };
    let maximum = if parent == Parent::Table { MAX_TABLE_ROWS } else { MAX_TABLE_COLUMNS };
    if items.len() > maximum || (parent == Parent::Table && items.is_empty()) {
        return fail();
    }
    let mut normalized = Vec::with_capacity(items.len());
    for child in items {
        normalized.push(normalize_node(child, parent, depth + 1, context)?);
    }
    if parent == Parent::Table {
        validate_table_geometry(&normalized)?;
    }
    Ok(Value::Array(normalized))
}

fn validate_table_geometry(rows: &[Value]) -> CResult<()> {
    // A bounded rolling row tracks remaining rowspan coverage. Never allocate
    // a width × height map from untrusted dimensions or span attributes.
    let mut occupancy = [0usize; MAX_TABLE_COLUMNS];
    let mut width = 0;
    for (row_index, row) in rows.iter().enumerate() {
        let cells = row["content"].as_array().ok_or(ContentError(INVALID_DOCUMENT_MESSAGE))?;
        let mut column = 0;
        for cell in cells {
            while column < MAX_TABLE_COLUMNS && occupancy[column] != 0 {
                column += 1;
            }
            // Attributes have already been canonicalised into bounded integers.
            let colspan = cell["attrs"]["colspan"].as_u64().unwrap() as usize;
            let rowspan = cell["attrs"]["rowspan"].as_u64().unwrap() as usize;
            let end = column + colspan;
            if end > MAX_TABLE_COLUMNS || rowspan > rows.len() - row_index {
                return fail();
            }
            if occupancy[column..end].iter().any(|remaining| *remaining != 0) {
                return fail();
            }
            occupancy[column..end].fill(rowspan);
            column = end;
        }
        let row_width = occupancy.iter().rposition(|remaining| *remaining != 0)
            .map_or(0, |column| column + 1);
        if row_index == 0 {
            width = row_width;
        }
        if width == 0 || row_width != width || occupancy[..width].contains(&0) {
            return fail();
        }
        for remaining in &mut occupancy[..width] {
            *remaining -= 1;
        }
    }
    Ok(())
}

fn normalize_inline_content(
    node: &Map<String, Value>,
    depth: usize,
    context: &mut Context,
) -> CResult<Option<Value>> {
    let Some(value) = node.get("content") else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail();
    };
    if items.len() > MAX_DOCUMENT_NODES {
        return fail();
    }
    if items.is_empty() {
        return Ok(None);
    }
    let mut normalized = Vec::with_capacity(items.len());
    for child in items {
        normalized.push(normalize_node(child, Parent::Inline, depth + 1, context)?);
    }
    Ok(Some(Value::Array(normalized)))
}

fn normalize_block_content(
    node: &Map<String, Value>,
    parent: Parent,
    depth: usize,
    context: &mut Context,
) -> CResult<Value> {
    let Some(Value::Array(items)) = node.get("content") else {
        return fail();
    };
    if items.is_empty() || items.len() > MAX_DOCUMENT_NODES {
        return fail();
    }
    let mut normalized = Vec::with_capacity(items.len());
    for child in items {
        normalized.push(normalize_node(child, parent, depth + 1, context)?);
    }
    Ok(Value::Array(normalized))
}

fn normalize_node(
    value: &Value,
    parent: Parent,
    depth: usize,
    context: &mut Context,
) -> CResult<Value> {
    if depth > MAX_DOCUMENT_DEPTH {
        return fail();
    }
    let node = as_record(value)?;
    let Some(node_type) = node.get("type").and_then(Value::as_str) else {
        return fail();
    };
    let Some(allowed_keys) = node_keys(node_type) else {
        return fail();
    };
    if !allowed_parent(node_type, parent) {
        return fail();
    }
    if !has_only_keys(node, allowed_keys) {
        return fail();
    }

    context.nodes += 1;
    if context.nodes > MAX_DOCUMENT_NODES {
        return fail();
    }

    match node_type {
        "text" => {
            let Some(text) = node.get("text").and_then(Value::as_str) else {
                return fail();
            };
            if text.is_empty() {
                return fail();
            }
            context.text_length += utf16_len(text);
            if context.text_length > MAX_DOCUMENT_TEXT {
                return fail();
            }
            if !js_trim(text).is_empty() {
                context.has_visible_content = true;
            }
            let marks = normalize_marks(node.get("marks"))?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("text".to_string()));
            out.insert("text".to_string(), Value::String(text.to_string()));
            if let Some(marks) = marks {
                out.insert("marks".to_string(), marks);
            }
            Ok(Value::Object(out))
        }
        "hardBreak" => {
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("hardBreak".to_string()));
            Ok(Value::Object(out))
        }
        "paragraph" => {
            let attrs = normalize_alignment_attrs(node.get("attrs"))?;
            let content = normalize_inline_content(node, depth, context)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("paragraph".to_string()));
            if let Some(attrs) = attrs {
                out.insert("attrs".to_string(), attrs);
            }
            if let Some(content) = content {
                out.insert("content".to_string(), content);
            }
            Ok(Value::Object(out))
        }
        "heading" => {
            let attrs = normalize_heading_attrs(node.get("attrs"))?;
            let content = normalize_inline_content(node, depth, context)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("heading".to_string()));
            out.insert("attrs".to_string(), attrs);
            if let Some(content) = content {
                out.insert("content".to_string(), content);
            }
            Ok(Value::Object(out))
        }
        "blockquote" => {
            let content = normalize_block_content(node, Parent::Blockquote, depth, context)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("blockquote".to_string()));
            out.insert("content".to_string(), content);
            Ok(Value::Object(out))
        }
        "bulletList" => {
            let content = normalize_block_content(node, Parent::BulletList, depth, context)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("bulletList".to_string()));
            out.insert("content".to_string(), content);
            Ok(Value::Object(out))
        }
        "orderedList" => {
            let attrs = normalize_ordered_list_attrs(node.get("attrs"))?;
            let content = normalize_block_content(node, Parent::OrderedList, depth, context)?;
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("orderedList".to_string()));
            if let Some(attrs) = attrs {
                out.insert("attrs".to_string(), attrs);
            }
            out.insert("content".to_string(), content);
            Ok(Value::Object(out))
        }
        "listItem" => {
            let content = normalize_block_content(node, Parent::ListItem, depth, context)?;
            let first_is_paragraph = content
                .as_array()
                .and_then(|items| items.first())
                .and_then(|first| first.get("type"))
                .and_then(Value::as_str)
                .map(|inner| inner == "paragraph")
                .unwrap_or(false);
            if !first_is_paragraph {
                return fail();
            }
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("listItem".to_string()));
            out.insert("content".to_string(), content);
            Ok(Value::Object(out))
        }
        "table" | "tableRow" => {
            let parent = if node_type == "table" { Parent::Table } else { Parent::TableRow };
            let content = normalize_table_children(node, parent, depth, context)?;
            Ok(json!({ "type": node_type, "content": content }))
        }
        "tableCell" | "tableHeader" => {
            let attrs = normalize_table_cell_attrs(node.get("attrs"))?;
            let content = normalize_block_content(node, Parent::TableCell, depth, context)?;
            Ok(json!({ "type": node_type, "attrs": attrs, "content": content }))
        }
        _ => fail(),
    }
}

/// Port of `normalizePostDocument`: validates and canonicalises an untrusted
/// TipTap JSON body, keeping only whitelisted nodes, marks, attributes and link
/// protocols while enforcing every resource bound before the body is stored.
pub fn normalize_post_document(value: &Value) -> Result<Value, ContentError> {
    validate_document_json_size(value)?;

    let document = as_record(value)?;
    if document.get("type").and_then(Value::as_str) != Some("doc") {
        return fail();
    }
    if !has_only_keys(document, &["type", "content"]) {
        return fail();
    }
    let Some(Value::Array(content)) = document.get("content") else {
        return fail();
    };
    if content.len() > MAX_DOCUMENT_NODES {
        return fail();
    }
    if content.is_empty() {
        return Err(ContentError(EMPTY_DOCUMENT_MESSAGE));
    }

    let mut context = Context {
        nodes: 0,
        text_length: 0,
        has_visible_content: false,
    };
    let mut normalized = Vec::with_capacity(content.len());
    for child in content {
        normalized.push(normalize_node(child, Parent::Doc, 1, &mut context)?);
    }
    if !context.has_visible_content {
        return Err(ContentError(EMPTY_DOCUMENT_MESSAGE));
    }

    let mut out = Map::new();
    out.insert("type".to_string(), Value::String("doc".to_string()));
    out.insert("content".to_string(), Value::Array(normalized));
    let document = Value::Object(out);
    // Default cell attributes can enlarge the input; persisted documents must
    // also fit the cap so subsequent reads and re-normalisation remain valid.
    validate_document_json_size(&document)?;
    Ok(document)
}

/// Validates an optional rich document such as a province description:
/// `null` clears the value and a structurally empty document is treated the
/// same way, so admins can blank a field with either form. Everything else
/// must satisfy the post whitelist and its resource bounds.
pub fn normalize_optional_document(value: &Value) -> Result<Option<Value>, ContentError> {
    if value.is_null() {
        return Ok(None);
    }
    match normalize_post_document(value) {
        Ok(document) => Ok(Some(document)),
        Err(ContentError(message)) if message == EMPTY_DOCUMENT_MESSAGE => Ok(None),
        Err(error) => Err(error),
    }
}

/// Converts a legacy plain-text description into the canonical paragraph-based
/// rich document: one paragraph per line, blank lines staying empty paragraphs,
/// CRLF/CR line endings normalised to LF. Tabs become spaces and control
/// characters the rich-text whitelist cannot represent are dropped, so the
/// result always survives `normalize_post_document` structurally.
pub fn plain_text_to_document(text: &str) -> Value {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let content = normalized
        .split('\n')
        .map(|line| {
            let visible: String = line
                .chars()
                .filter_map(|character| match character {
                    '\t' => Some(' '),
                    character if character <= '\u{001F}' || character == '\u{007F}' => None,
                    character => Some(character),
                })
                .collect();
            if js_trim(&visible).is_empty() {
                return json!({ "type": "paragraph" });
            }
            json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": visible }],
            })
        })
        .collect::<Vec<_>>();
    json!({ "type": "doc", "content": content })
}

/// Storage value of a legacy plain-text description: `None` when the text has
/// no visible content (the historical "no description" sentinel), otherwise
/// the serialised paragraph document.
pub fn legacy_description_json(text: &str) -> Result<Option<String>, String> {
    let document = plain_text_to_document(text);
    let has_visible_content = document
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|lines| lines.iter().any(|line| line.get("content").is_some()));
    if !has_visible_content {
        return Ok(None);
    }
    serde_json::to_string(&document)
        .map(Some)
        .map_err(|error| format!("Cannot serialise the converted description: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paragraph(text: &str) -> Value {
        json!({ "type": "paragraph", "content": [{ "type": "text", "text": text }] })
    }

    fn table_cell(node_type: &str, attrs: Value, blocks: Vec<Value>) -> Value {
        json!({ "type": node_type, "attrs": attrs, "content": blocks })
    }

    fn table_document(rows: Vec<Vec<Value>>) -> Value {
        json!({
            "type": "doc",
            "content": [{
                "type": "table",
                "content": rows.into_iter().map(|cells| {
                    json!({ "type": "tableRow", "content": cells })
                }).collect::<Vec<_>>()
            }]
        })
    }

    fn simple_table() -> Value {
        table_document(vec![
            vec![
                table_cell("tableHeader", json!({}), vec![paragraph("Название")]),
                table_cell("tableHeader", json!({}), vec![paragraph("Год")]),
            ],
            vec![
                table_cell("tableCell", json!({}), vec![paragraph("Рязань")]),
                table_cell("tableCell", json!({}), vec![paragraph("1897")]),
            ],
        ])
    }

    #[test]
    fn explicit_left_block_alignment_overrides_cell_alignment_and_survives_roundtrip() {
        for align in ["center", "right"] {
            let mut paragraph_left = paragraph("Слева");
            paragraph_left["attrs"] = json!({ "textAlign": "left" });
            let heading_left = json!({
                "type": "heading",
                "attrs": { "level": 2, "textAlign": "left" },
                "content": [{ "type": "text", "text": "Заголовок слева" }],
            });
            let mut document = table_document(vec![vec![table_cell(
                "tableCell",
                json!({ "align": align }),
                vec![
                    paragraph_left.clone(),
                    heading_left.clone(),
                    json!({ "type": "paragraph", "attrs": { "textAlign": null } }),
                    json!({ "type": "heading", "attrs": { "level": 3, "textAlign": null } }),
                ],
            )]]);
            // Explicit left is preserved outside a table too: no implicit
            // inherited-context special cases enter document canonicalisation.
            document["content"].as_array_mut().unwrap().extend([
                paragraph_left,
                heading_left,
            ]);
            let normalized = normalize_post_document(&document).unwrap();
            let blocks = &normalized["content"][0]["content"][0]["content"][0]["content"];
            assert_eq!(blocks[0]["attrs"]["textAlign"], "left");
            assert_eq!(blocks[1]["attrs"]["textAlign"], "left");
            assert!(blocks[2].get("attrs").is_none());
            assert_eq!(blocks[3]["attrs"], json!({ "level": 3 }));
            assert_eq!(normalized["content"][1]["attrs"]["textAlign"], "left");
            assert_eq!(normalized["content"][2]["attrs"]["textAlign"], "left");
            assert_eq!(normalize_post_document(&normalized).unwrap(), normalized);
        }
    }

    #[test]
    fn table_defaults_are_full_canonical_attributes_and_idempotent() {
        let mut document = simple_table();
        document["content"][0]["content"][0]["content"][0].as_object_mut().unwrap().remove("attrs");
        document["content"][0]["content"][0]["content"][1]["attrs"] = json!({
            "colspan": 1.0, "rowspan": 1.0, "colwidth": [0.0], "align": null,
        });
        let normalized = normalize_post_document(&document).unwrap();
        assert_eq!(normalized["content"][0]["content"][0]["content"][0]["attrs"], json!({
            "colspan": 1, "rowspan": 1, "colwidth": null, "align": null,
        }));
        assert_eq!(normalized["content"][0]["content"][0]["content"][1]["attrs"]["colwidth"], json!([0]));
        assert_eq!(normalize_post_document(&normalized).unwrap(), normalized);
        assert!(document["content"][0]["content"][0]["content"][0].get("attrs").is_none());
    }

    #[test]
    fn table_spans_empty_covered_rows_nested_blocks_and_marks_survive() {
        let mut marked = paragraph("Источник");
        marked["content"][0]["marks"] = json!([
            { "type": "bold" },
            { "type": "link", "attrs": { "href": "/source", "target": "_blank" } },
            { "type": "textStyle", "attrs": { "color": "#ABC" } },
        ]);
        let document = table_document(vec![
            vec![
                table_cell("tableHeader", json!({
                    "colspan": 2, "rowspan": 2, "colwidth": [0, 10_000], "align": "center",
                }), vec![marked]),
                table_cell("tableHeader", json!({ "align": "right" }), vec![
                    json!({ "type": "heading", "attrs": { "level": 2 }, "content": [
                        { "type": "text", "text": "Год" },
                    ]}),
                ]),
            ],
            vec![table_cell("tableCell", json!({ "align": "left" }), vec![
                json!({ "type": "paragraph" }),
                simple_table()["content"][0].clone(),
                json!({ "type": "bulletList", "content": [
                    { "type": "listItem", "content": [paragraph("Список")] },
                ]}),
            ])],
            vec![table_cell("tableCell", json!({ "colspan": 3, "rowspan": 2 }), vec![paragraph("Итого")])],
            vec![],
        ]);
        let normalized = normalize_post_document(&document).unwrap();
        let header = &normalized["content"][0]["content"][0]["content"][0];
        assert_eq!(header["attrs"], json!({
            "colspan": 2, "rowspan": 2, "colwidth": [0, 10_000], "align": "center",
        }));
        assert_eq!(header["content"][0]["content"][0]["marks"][1]["attrs"], json!({ "href": "/source" }));
        assert_eq!(header["content"][0]["content"][0]["marks"][2]["attrs"]["color"], "#abc");
        assert_eq!(normalized["content"][0]["content"][3]["content"], json!([]));
        assert_eq!(normalize_post_document(&normalized).unwrap(), normalized);
    }

    #[test]
    fn table_rejects_untrusted_attributes_and_wrong_structure() {
        for attrs in [
            json!(null), json!({ "style": "color:red" }), json!({ "textAlign": "center" }),
            json!({ "colspan": null }), json!({ "colspan": 0 }), json!({ "colspan": 101 }),
            json!({ "colspan": 1.5 }), json!({ "colspan": "1" }), json!({ "colspan": true }),
            json!({ "rowspan": null }), json!({ "rowspan": -1 }), json!({ "rowspan": 101 }),
            json!({ "rowspan": 1.5 }), json!({ "rowspan": "1" }), json!({ "rowspan": false }),
            json!({ "colwidth": [] }), json!({ "colwidth": [1, 2] }), json!({ "colwidth": "2" }),
            json!({ "colwidth": [null] }), json!({ "colwidth": [-1] }),
            json!({ "colwidth": [10_001] }), json!({ "colwidth": [1.5] }),
            json!({ "colwidth": ["1"] }), json!({ "colwidth": [true] }),
            json!({ "align": "justify" }), json!({ "align": "CENTER" }),
            json!({ "align": "left;position:fixed" }), json!({ "align": 1 }),
        ] {
            let mut document = simple_table();
            document["content"][0]["content"][0]["content"][0]["attrs"] = attrs.clone();
            assert_eq!(normalize_post_document(&document).unwrap_err().0, INVALID_DOCUMENT_MESSAGE, "{attrs}");
        }
        for (pointer, replacement) in [
            ("/content/0/attrs", json!({})),
            ("/content/0/content/0/attrs", json!(null)),
            ("/content/0/content/0/content/0/content", json!([])),
            ("/content/0/content/0/content/0/content", json!([{ "type": "text", "text": "x" }])),
            ("/content/0/content/0/content/0/marks", json!([])),
            ("/content/0/content/0/content/0/type", json!("paragraph")),
            ("/content/0/content/0/type", json!("tableCell")),
            ("/content/0/content/0/content/0/content/0/type", json!("tableRow")),
            ("/content/0/content", json!(null)),
            ("/content/0/content/0/content", json!(null)),
        ] {
            let mut document = simple_table();
            if pointer == "/content/0/attrs" {
                document["content"][0]["attrs"] = replacement;
            } else if pointer == "/content/0/content/0/attrs" {
                document["content"][0]["content"][0]["attrs"] = replacement;
            } else if pointer.ends_with("/marks") {
                document["content"][0]["content"][0]["content"][0]["marks"] = replacement;
            } else {
                *document.pointer_mut(pointer).unwrap() = replacement;
            }
            assert_eq!(normalize_post_document(&document).unwrap_err().0, INVALID_DOCUMENT_MESSAGE, "{pointer}");
        }
        for node in ["tableRow", "tableCell", "tableHeader"] {
            assert!(normalize_post_document(&json!({
                "type": "doc", "content": [{ "type": node, "content": [paragraph("x")] }],
            })).is_err());
        }
    }

    #[test]
    fn table_rejects_overlapping_ragged_holey_and_out_of_bounds_grids() {
        let plain = || table_cell("tableCell", json!({}), vec![paragraph("x")]);
        let span = |colspan, rowspan| table_cell("tableCell", json!({
            "colspan": colspan, "rowspan": rowspan,
        }), vec![paragraph("x")]);
        for document in [
            table_document(vec![]),
            table_document(vec![vec![]]),
            table_document(vec![vec![plain(), plain()], vec![plain()]]),
            table_document(vec![vec![plain()], vec![plain(), plain()]]),
            table_document(vec![vec![span(1, 2)]]),
            table_document(vec![vec![plain(), span(1, 2)], vec![]]),
            table_document(vec![vec![plain(), span(1, 2)], vec![span(2, 1)]]),
            table_document(vec![vec![span(100, 1), plain()]]),
            table_document(vec![vec![plain(); 101]]),
            table_document(vec![vec![plain()]; 101]),
        ] {
            assert_eq!(normalize_post_document(&document).unwrap_err().0, INVALID_DOCUMENT_MESSAGE, "{document}");
        }
        // Maximum dimensions are possible without materialising a 100×100 map.
        let mut rows = vec![vec![span(100, 100)]];
        rows.extend(vec![vec![]; 99]);
        let normalized = normalize_post_document(&table_document(rows)).unwrap();
        assert_eq!(normalized["content"][0]["content"].as_array().unwrap().len(), 100);
    }

    #[test]
    fn table_blank_content_and_existing_resource_limits_are_unchanged() {
        let blank = table_document(vec![vec![
            table_cell("tableCell", json!({}), vec![json!({ "type": "paragraph" })]),
        ]]);
        assert_eq!(normalize_post_document(&blank).unwrap_err().0, EMPTY_DOCUMENT_MESSAGE);
        assert_eq!(normalize_optional_document(&blank).unwrap(), None);
        let mut invalid_blank = blank.clone();
        invalid_blank["content"][0]["content"][0]["content"][0]["attrs"]["colspan"] = json!(0);
        assert_eq!(normalize_optional_document(&invalid_blank).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);

        let nest = |count| {
            let mut block = paragraph("x");
            for _ in 0..count {
                block = table_document(vec![vec![
                    table_cell("tableCell", json!({}), vec![block]),
                ]])["content"][0].clone();
            }
            json!({ "type": "doc", "content": [block] })
        };
        assert!(normalize_post_document(&nest(6)).is_ok());
        assert_eq!(normalize_post_document(&nest(7)).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);

        let long = table_document(vec![vec![
            table_cell("tableCell", json!({}), vec![paragraph(&"x".repeat(MAX_DOCUMENT_TEXT + 1))]),
        ]]);
        assert_eq!(normalize_post_document(&long).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);

        let mut crowded = simple_table();
        crowded["content"].as_array_mut().unwrap().extend(
            (0..MAX_DOCUMENT_NODES - 1).map(|_| json!({ "type": "paragraph" })),
        );
        assert!(serde_json::to_string(&crowded).unwrap().len() < MAX_POST_DOCUMENT_JSON_CHARACTERS);
        assert_eq!(normalize_post_document(&crowded).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);

        let mut oversized = simple_table();
        oversized["content"].as_array_mut().unwrap().extend(
            (0..4_500).map(|_| json!({ "type": "paragraph", "attrs": { "textAlign": null } })),
        );
        assert_eq!(normalize_post_document(&oversized).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);
    }

    #[test]
    fn json_size_counter_matches_utf16_serialization_and_exact_boundary() {
        for value in [
            json!({ "text": "Рязань 😀\n\"\\", "nested": [null, true, 1.5] }),
            json!("😀".repeat(MAX_POST_DOCUMENT_JSON_CHARACTERS / 2 - 1)),
        ] {
            let mut counter = JsonCharacterCount::default();
            serde_json::to_writer(&mut counter, &value).unwrap();
            assert_eq!(counter.0, utf16_len(&serde_json::to_string(&value).unwrap()));
            assert!(validate_document_json_size(&value).is_ok());
        }
        assert!(validate_document_json_size(&json!(
            "😀".repeat(MAX_POST_DOCUMENT_JSON_CHARACTERS / 2)
        )).is_err());
    }

    #[test]
    fn canonical_table_json_also_must_fit_the_serialized_limit() {
        let cell = table_cell("tableCell", json!({}), vec![json!({ "type": "paragraph" })]);
        let mut document = table_document(vec![vec![cell; 20]; 100]);
        document["content"][0]["content"][0]["content"][0]["content"][0] = paragraph("видимый текст");
        assert!(utf16_len(&serde_json::to_string(&document).unwrap()) < MAX_POST_DOCUMENT_JSON_CHARACTERS);
        // Fewer than 5,000 nodes and valid 100×20 geometry: only canonical JSON
        // growth (full four attrs on every cell) makes this document invalid.
        assert_eq!(normalize_post_document(&document).unwrap_err().0, INVALID_DOCUMENT_MESSAGE);

        let canonical = normalize_post_document(&simple_table()).unwrap();
        assert_eq!(normalize_post_document(&canonical).unwrap(), canonical);
    }

    #[test]
    fn accepts_whitelisted_document() {
        let document = json!({
            "type": "doc",
            "content": [
                paragraph("Привет"),
                {
                    "type": "heading",
                    "attrs": { "level": 2, "textAlign": "center" },
                    "content": [{ "type": "text", "text": "Заголовок", "marks": [
                        { "type": "bold" },
                        { "type": "link", "attrs": { "href": "https://example.com", "target": "_blank" } }
                    ]}]
                },
                { "type": "bulletList", "content": [
                    { "type": "listItem", "content": [paragraph("раз")] }
                ]}
            ]
        });
        let normalized = normalize_post_document(&document).unwrap();
        assert_eq!(normalized["type"], "doc");
        assert_eq!(normalized["content"][1]["attrs"]["textAlign"], "center");
        // target is validated but never persisted, like the TypeScript store.
        assert_eq!(
            normalized["content"][1]["content"][0]["marks"][1]["attrs"],
            json!({ "href": "https://example.com" })
        );
    }

    #[test]
    fn rejects_unknown_nodes_and_marks() {
        let bad_node = json!({ "type": "doc", "content": [{ "type": "image", "src": "x" }] });
        assert_eq!(
            normalize_post_document(&bad_node).unwrap_err().0,
            INVALID_DOCUMENT_MESSAGE
        );
        let bad_mark = json!({ "type": "doc", "content": [paragraph("x")] });
        let mut bad_mark = bad_mark;
        bad_mark["content"][0]["content"][0]["marks"] = json!([{ "type": "strike", "attrs": { "color": "#fff" } }]);
        assert!(normalize_post_document(&bad_mark).is_err());
    }

    #[test]
    fn rejects_javascript_links_and_protocol_relative_paths() {
        for href in ["javascript:alert(1)", "//evil.example", "mailto:a@b.c", "ftp://x"] {
            let document = json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "link", "attrs": { "href": href } }]
                }]}]
            });
            assert!(
                normalize_post_document(&document).is_err(),
                "{href} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_and_invisible_documents() {
        let empty = json!({ "type": "doc", "content": [] });
        assert_eq!(
            normalize_post_document(&empty).unwrap_err().0,
            EMPTY_DOCUMENT_MESSAGE
        );
        let invisible = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "   " }] }]
        });
        assert_eq!(
            normalize_post_document(&invisible).unwrap_err().0,
            EMPTY_DOCUMENT_MESSAGE
        );
        let blank_only = json!({ "type": "doc", "content": [{ "type": "paragraph" }] });
        assert_eq!(
            normalize_post_document(&blank_only).unwrap_err().0,
            EMPTY_DOCUMENT_MESSAGE
        );
    }

    #[test]
    fn enforces_text_node_bounds_and_lists() {
        let long_text = "я".repeat(MAX_DOCUMENT_TEXT + 1);
        let document = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": long_text }] }]
        });
        assert!(normalize_post_document(&document).is_err());

        // A list item must start with a paragraph.
        let nested = json!({
            "type": "doc",
            "content": [{ "type": "bulletList", "content": [
                { "type": "listItem", "content": [{ "type": "bulletList", "content": [
                    { "type": "listItem", "content": [paragraph("x")] }
                ]}]}
            ]}]
        });
        assert!(normalize_post_document(&nested).is_err());
    }

    #[test]
    fn ordered_list_start_accepts_integral_floats() {
        let list = |start: Value| {
            json!({
                "type": "doc",
                "content": [{
                    "type": "orderedList",
                    "attrs": { "start": start },
                    "content": [{ "type": "listItem", "content": [paragraph("x")] }]
                }]
            })
        };

        // Integral floats parse exactly like integers once serialised.
        let document: Value = serde_json::from_str(
            r#"{"type":"doc","content":[{"type":"orderedList","attrs":{"start":2.0},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}]}]}"#,
        )
        .unwrap();
        let normalized = normalize_post_document(&document).unwrap();
        assert_eq!(normalized["content"][0]["attrs"]["start"], json!(2));

        let normalized = normalize_post_document(&list(json!(1_000_000.0))).unwrap();
        assert_eq!(
            normalized["content"][0]["attrs"]["start"],
            json!(1_000_000)
        );

        // start === 1 drops the attribute entirely.
        let normalized = normalize_post_document(&list(json!(1.0))).unwrap();
        assert!(normalized["content"][0].get("attrs").is_none());

        // Non-integral, out-of-range and non-number values stay invalid.
        for invalid in [json!(2.5), json!(0.0), json!(1_000_001.0), json!("3"), json!(true)] {
            assert!(
                normalize_post_document(&list(invalid.clone())).is_err(),
                "{invalid} must be rejected"
            );
        }
    }

    #[test]
    fn normalizes_colors_and_alignment() {
        let document = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": { "textAlign": "justify" },
                "content": [{
                    "type": "text",
                    "text": "цвет",
                    "marks": [{ "type": "textStyle", "attrs": { "color": "rgb(255, 0, 16)" } }]
                }]
            }]
        });
        let normalized = normalize_post_document(&document).unwrap();
        assert_eq!(normalized["content"][0]["attrs"]["textAlign"], "justify");
        assert_eq!(
            normalized["content"][0]["content"][0]["marks"][0]["attrs"]["color"],
            "#ff0010"
        );

        let dropped = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{
                "type": "text",
                "text": "x",
                "marks": [{ "type": "textStyle", "attrs": { "color": "" } }]
            }]}]
        });
        let normalized = normalize_post_document(&dropped).unwrap();
        assert!(normalized["content"][0]["content"][0].get("marks").is_none());
    }

    #[test]
    fn optional_documents_treat_null_and_blank_as_cleared() {
        assert!(normalize_optional_document(&Value::Null).unwrap().is_none());
        for empty in [
            json!({ "type": "doc", "content": [] }),
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "   " }] }]
            }),
        ] {
            assert!(normalize_optional_document(&empty).unwrap().is_none(), "{empty}");
        }

        let document = normalize_optional_document(&json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
        }))
        .unwrap()
        .expect("a visible document survives");
        assert_eq!(document["content"][0]["content"][0]["text"], "x");

        // Strings and unsafe documents are rejected, never treated as empty.
        for invalid in [
            json!("просто строка"),
            json!(42),
            json!({ "type": "doc", "content": [{ "type": "image", "src": "x" }] }),
        ] {
            assert!(normalize_optional_document(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn plain_text_converts_to_paragraphs_per_line() {
        let document = plain_text_to_document("Первая\n\nВторая\tстрока");
        assert_eq!(
            document,
            json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "Первая" }] },
                    { "type": "paragraph" },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "Вторая строка" }] }
                ]
            })
        );
        // The built document is structurally valid for the whitelist.
        assert!(normalize_post_document(&document).is_ok());

        // CRLF/CR become LF; other control characters cannot be represented.
        let converted = plain_text_to_document("one\r\ntwo\rthree\u{0007}");
        assert_eq!(converted["content"].as_array().unwrap().len(), 3);
        assert_eq!(converted["content"][2]["content"][0]["text"], "three");

        // A trailing newline keeps its empty last line.
        let trailing = plain_text_to_document("one\n");
        assert_eq!(trailing["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn legacy_descriptions_only_keep_visible_text() {
        assert!(legacy_description_json("").unwrap().is_none());
        assert!(legacy_description_json(" \n\t\n ").unwrap().is_none());
        assert!(legacy_description_json("\r\n").unwrap().is_none());

        let stored = legacy_description_json("строка \"в кавычках\" \\ слэшем\n\nвторая")
            .unwrap()
            .expect("visible text converts");
        let parsed: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            parsed,
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "строка \"в кавычках\" \\ слэшем" }]
                    },
                    { "type": "paragraph" },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "вторая" }] }
                ]
            })
        );
        assert!(normalize_post_document(&parsed).is_ok());
    }
}
