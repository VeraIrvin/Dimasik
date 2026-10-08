use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value};

use crate::content::{normalize_post_document, ContentError};
use crate::error::{ApiError, ApiResult};
use crate::geo::{CanonicalProvinces, GeoRuntime};
use crate::util::{
    effective_settlement_url, is_valid_settlement_url, is_valid_slug, iso_now, js_collapse_whitespace,
    js_number, js_trim, parse_js_date_ms, random_id, ru_compare, utf16_len, SETTLEMENT_URL_PREFIX,
};

pub const MAX_DESCRIPTION_LENGTH: usize = 20_000;
pub const MAX_TITLE_LENGTH: usize = 1_000;
pub const MAX_ID_LENGTH: usize = 100;
pub const MAX_SETTLEMENT_NAME_LENGTH: usize = 200;
pub const MAX_YEAR_LENGTH: usize = 100;
pub const MAX_ARCHIVE_REFERENCE_LENGTH: usize = 300;
pub const MAX_STORED_SETTING_NAME_LENGTH: usize = 200;
pub const MAX_SETTINGS_NAME_CHARACTERS: usize = 80;
pub const MAX_SETTINGS_ITEMS: usize = 100;
pub const MAX_SETTINGS_STORED_BYTES: usize = 65_536;

pub const POST_CATEGORIES: [&str; 4] = [
    "Статья",
    "Персона",
    "Ссылка на источник",
    "Источник с индексацией",
];
pub const DEFAULT_SETTLEMENT_TYPES: [&str; 3] = ["Город", "Село", "Деревня"];

pub const DEFAULT_ABOUT_PARAGRAPHS: [&str; 5] = [
    "Это прототип интерфейса для изучения губерний Российской империи и будущей работы с архивными документами.",
    "Карта показывает историческую реконструкцию границ 76 губерний по состоянию на 1897 год. Изначально десять опубликованных губерний выделены бежевым и представлены в списке поверх карты. Наведение на губернию в списке или на карте подсвечивает обе её формы представления; клик открывает страницу губернии. Остальные губернии показаны бледным контекстом, не подсвечиваются и не открываются по клику. Другие территории того же среза образуют нейтральный фон без границ и подписей. Современная географическая подложка не используется.",
    "Вход администратора расположен на главной странице. Администратор может добавить губернию из исторического слоя в список доступных, изменить адрес и описание её страницы или удалить страницу. Полигон удалённой губернии остаётся нейтральным контекстом карты.",
    "Сообщения на страницах губерний доступны всем посетителям. Боковой список заголовков помогает перейти к нужному сообщению; администратор может публиковать, редактировать и удалять сообщения с форматированным текстом.",
    "На странице каждой опубликованной губернии под её названием есть карта границ уездов и округов. Список слева и карта подсвечивают одну и ту же единицу; нажатие приближает к ней, но отдельные страницы уездов пока не создаются.",
];

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::bad_request(message)
}

fn optional(value: Option<&Value>) -> Option<&Value> {
    value.filter(|inner| !inner.is_null())
}

pub fn validate_slug(value: &Value) -> ApiResult<String> {
    match value.as_str() {
        Some(slug) if is_valid_slug(slug) => Ok(slug.to_string()),
        _ => Err(bad(
            "Slug должен содержать только строчные латинские буквы, цифры и одиночные дефисы.",
        )),
    }
}

pub fn validate_description(value: &Value) -> ApiResult<String> {
    match value.as_str() {
        Some(description) if utf16_len(description) <= MAX_DESCRIPTION_LENGTH => {
            Ok(description.to_string())
        }
        _ => Err(bad(format!(
            "Описание должно быть строкой не длиннее {MAX_DESCRIPTION_LENGTH} символов."
        ))),
    }
}

pub fn validate_post_title(value: &Value) -> ApiResult<String> {
    let Some(raw) = value.as_str() else {
        return Err(bad("Заголовок должен быть строкой."));
    };
    let title = js_trim(raw);
    if title.is_empty() {
        return Err(bad("Заголовок не должен быть пустым."));
    }
    if utf16_len(title) > MAX_TITLE_LENGTH {
        return Err(bad(format!(
            "Заголовок должен быть не длиннее {MAX_TITLE_LENGTH} символов."
        )));
    }
    Ok(title.to_string())
}

pub fn validate_post_body(value: &Value) -> ApiResult<Value> {
    normalize_post_document(value).map_err(|ContentError(message)| bad(message))
}

fn format_options(options: &[String]) -> String {
    options
        .iter()
        .map(|option| format!("«{option}»"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn validate_post_category(value: &Value, allowed: &[String]) -> ApiResult<String> {
    match value.as_str() {
        Some(category) if allowed.iter().any(|allowed| allowed == category) => {
            Ok(category.to_string())
        }
        _ => Err(bad(format!(
            "Категория должна быть одной из: {}.",
            format_options(allowed)
        ))),
    }
}

pub fn validate_settlement_type(value: &Value, allowed: &[String]) -> ApiResult<String> {
    match value.as_str() {
        Some(settlement_type) if allowed.iter().any(|allowed| allowed == settlement_type) => {
            Ok(settlement_type.to_string())
        }
        _ => Err(bad(format!(
            "Тип населённого пункта должен быть одним из: {}.",
            format_options(allowed)
        ))),
    }
}

pub fn validate_settlement_name(value: &Value) -> ApiResult<String> {
    let Some(raw) = value.as_str() else {
        return Err(bad("Название населённого пункта должно быть строкой."));
    };
    let name = js_trim(raw);
    if name.is_empty() {
        return Err(bad("Название населённого пункта не должно быть пустым."));
    }
    if utf16_len(name) > MAX_SETTLEMENT_NAME_LENGTH {
        return Err(bad(format!(
            "Название населённого пункта должно быть не длиннее {MAX_SETTLEMENT_NAME_LENGTH} символов."
        )));
    }
    Ok(name.to_string())
}

pub fn validate_settlement_url(value: &Value) -> ApiResult<String> {
    match value.as_str() {
        Some(url) if is_valid_settlement_url(url) => Ok(url.to_string()),
        _ => Err(bad(
            "Адрес страницы должен начинаться с «/naselennyy-punkt/» и содержать только строчные латинские буквы, цифры и одиночные дефисы.",
        )),
    }
}

/// Parses the admin-supplied "широта, долгота" string into (latitude, longitude).
pub fn parse_coordinates(value: &Value) -> ApiResult<(f64, f64)> {
    let Some(raw) = value.as_str() else {
        return Err(bad("Координаты должны быть строкой вида «широта, долгота»."));
    };
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 2 {
        return Err(bad("Координаты должны содержать широту и долготу через запятую."));
    }
    let latitude_source = js_trim(parts[0]);
    let longitude_source = js_trim(parts[1]);
    let latitude = js_number(latitude_source);
    let longitude = js_number(longitude_source);
    if latitude_source.is_empty()
        || longitude_source.is_empty()
        || !latitude.is_finite()
        || !longitude.is_finite()
    {
        return Err(bad("Широта и долгота должны быть числами."));
    }
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(bad("Координаты выходят за допустимые пределы."));
    }
    Ok((latitude, longitude))
}

pub fn normalize_optional_reference_id(
    value: Option<&Value>,
    label: &str,
) -> ApiResult<Option<String>> {
    let Some(value) = optional(value) else {
        return Ok(None);
    };
    let Some(raw) = value.as_str() else {
        return Err(bad(format!("Поле «{label}» должно быть строкой.")));
    };
    let trimmed = js_trim(raw);
    if trimmed.is_empty() {
        return Ok(None);
    }
    if utf16_len(trimmed) > MAX_ID_LENGTH {
        return Err(bad(format!(
            "Поле «{label}» должно быть не длиннее {MAX_ID_LENGTH} символов."
        )));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn normalize_optional_text(
    value: Option<&Value>,
    label: &str,
    max_length: usize,
) -> ApiResult<String> {
    let Some(value) = optional(value) else {
        return Ok(String::new());
    };
    let Some(raw) = value.as_str() else {
        return Err(bad(format!("Поле «{label}» должно быть строкой.")));
    };
    let trimmed = js_trim(raw);
    if utf16_len(trimmed) > max_length {
        return Err(bad(format!(
            "Поле «{label}» должно быть не длиннее {max_length} символов."
        )));
    }
    Ok(trimmed.to_string())
}

#[derive(Clone, Debug)]
pub struct SettingsLists {
    pub categories: Vec<String>,
    pub settlement_types: Vec<String>,
}

pub fn default_settings_lists() -> SettingsLists {
    SettingsLists {
        categories: POST_CATEGORIES.iter().map(|value| value.to_string()).collect(),
        settlement_types: DEFAULT_SETTLEMENT_TYPES
            .iter()
            .map(|value| value.to_string())
            .collect(),
    }
}

fn normalize_setting_name(value: &Value) -> Result<String, String> {
    let Some(raw) = value.as_str() else {
        return Err("Введите название текстом.".to_string());
    };
    let name = js_collapse_whitespace(raw);
    if name.is_empty() {
        return Err("Введите название.".to_string());
    }
    if utf16_len(&name) > MAX_SETTINGS_NAME_CHARACTERS {
        return Err(format!(
            "Название не должно превышать {MAX_SETTINGS_NAME_CHARACTERS} символов."
        ));
    }
    Ok(name)
}

fn parse_settings_list(value: &Value, label: &str) -> Result<Vec<String>, String> {
    let Value::Array(items) = value else {
        return Err(format!("Site settings {label} list has an unsupported format."));
    };
    if items.len() > MAX_SETTINGS_ITEMS {
        return Err(format!("Site settings {label} list has an unsupported format."));
    }
    let mut result: Vec<String> = Vec::new();
    for item in items {
        let name = normalize_setting_name(item)
            .map_err(|_| format!("Site settings {label} list contains an invalid value."))?;
        if result.iter().any(|existing| existing == &name) {
            return Err(format!(
                "Site settings {label} list contains a duplicate value."
            ));
        }
        result.push(name);
    }
    Ok(result)
}

pub fn load_settings_lists(connection: &Connection) -> ApiResult<SettingsLists> {
    let stored: Option<(String, String)> = connection
        .query_row(
            "SELECT categories_json, settlement_types_json FROM site_settings WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((categories_json, settlement_types_json)) = stored else {
        return Ok(default_settings_lists());
    };

    let categories_value: Value = serde_json::from_str(&categories_json)
        .map_err(|error| ApiError::internal(format!("Site settings storage contains invalid JSON: {error}")))?;
    let types_value: Value = serde_json::from_str(&settlement_types_json)
        .map_err(|error| ApiError::internal(format!("Site settings storage contains invalid JSON: {error}")))?;
    let categories = parse_settings_list(&categories_value, "categories").map_err(ApiError::internal)?;
    let settlement_types =
        parse_settings_list(&types_value, "settlement types").map_err(ApiError::internal)?;
    Ok(SettingsLists {
        categories,
        settlement_types,
    })
}

pub fn settings_value(lists: &SettingsLists) -> Value {
    serde_json::json!({
        "categories": lists.categories,
        "settlementTypes": lists.settlement_types,
    })
}

fn store_settings_lists(connection: &Connection, lists: &SettingsLists) -> ApiResult<()> {
    let envelope = serde_json::json!({
        "version": 1,
        "categories": lists.categories,
        "settlementTypes": lists.settlement_types,
    });
    let source = format!("{}\n", serde_json::to_string(&envelope)?);
    if source.len() > MAX_SETTINGS_STORED_BYTES {
        return Err(bad("Список настроек достиг предельного размера."));
    }
    connection.execute(
        "INSERT INTO site_settings (id, categories_json, settlement_types_json)\n\
         VALUES (1, ?1, ?2)\n\
         ON CONFLICT(id) DO UPDATE SET\n\
           categories_json = excluded.categories_json,\n\
           settlement_types_json = excluded.settlement_types_json",
        params![
            serde_json::to_string(&lists.categories)?,
            serde_json::to_string(&lists.settlement_types)?
        ],
    )?;
    Ok(())
}

pub fn get_settings_value(connection: &Connection) -> ApiResult<Value> {
    let lists = load_settings_lists(connection)?;
    Ok(settings_value(&lists))
}

fn validate_kind(kind: &str) -> ApiResult<()> {
    if kind == "categories" || kind == "settlementTypes" {
        Ok(())
    } else {
        Err(bad("Неизвестный список настроек."))
    }
}

fn list_mut<'a>(lists: &'a mut SettingsLists, kind: &str) -> ApiResult<&'a mut Vec<String>> {
    match kind {
        "categories" => Ok(&mut lists.categories),
        "settlementTypes" => Ok(&mut lists.settlement_types),
        _ => Err(bad("Неизвестный список настроек.")),
    }
}

pub fn add_site_setting(
    connection: &mut Connection,
    kind: &str,
    value: &Value,
) -> ApiResult<Value> {
    validate_kind(kind)?;
    let name = normalize_setting_name(value).map_err(bad)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut lists = load_settings_lists(&transaction)?;
    let values = list_mut(&mut lists, kind)?;
    if values.iter().any(|existing| existing == &name) {
        return Err(bad("Такое название уже есть в списке."));
    }
    if values.len() >= MAX_SETTINGS_ITEMS {
        return Err(bad("Список настроек достиг предельного размера."));
    }
    values.push(name);
    store_settings_lists(&transaction, &lists)?;
    transaction.commit()?;
    Ok(settings_value(&lists))
}

pub fn rename_site_setting(
    connection: &mut Connection,
    kind: &str,
    original_name: &Value,
    value: &Value,
) -> ApiResult<Value> {
    validate_kind(kind)?;
    let Some(original_name) = original_name.as_str() else {
        return Err(bad("Исходное название не найдено."));
    };
    let name = normalize_setting_name(value).map_err(bad)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut lists = load_settings_lists(&transaction)?;
    let values = list_mut(&mut lists, kind)?;
    let Some(index) = values.iter().position(|existing| existing == original_name) else {
        return Err(bad("Исходное название не найдено."));
    };
    if name != original_name && values.iter().any(|existing| existing == &name) {
        return Err(bad("Такое название уже есть в списке."));
    }
    values[index] = name;
    store_settings_lists(&transaction, &lists)?;
    transaction.commit()?;
    Ok(settings_value(&lists))
}

pub fn delete_site_setting(
    connection: &mut Connection,
    kind: &str,
    name: &Value,
) -> ApiResult<Value> {
    validate_kind(kind)?;
    let Some(name) = name.as_str() else {
        return Err(bad("Название не найдено."));
    };
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut lists = load_settings_lists(&transaction)?;
    let values = list_mut(&mut lists, kind)?;
    let Some(index) = values.iter().position(|existing| existing == name) else {
        return Err(bad("Название не найдено."));
    };
    values.remove(index);
    store_settings_lists(&transaction, &lists)?;
    transaction.commit()?;
    Ok(settings_value(&lists))
}

fn default_about_document() -> Value {
    let content = DEFAULT_ABOUT_PARAGRAPHS
        .iter()
        .map(|text| {
            serde_json::json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": text }],
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "type": "doc", "content": content })
}

pub fn get_about_content(connection: &Connection) -> ApiResult<Value> {
    let stored: Option<String> = connection
        .query_row("SELECT body_json FROM about_content WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(stored) = stored else {
        return Ok(default_about_document());
    };
    let value: Value = serde_json::from_str(&stored).map_err(|error| {
        ApiError::internal(format!("About content storage contains invalid JSON: {error}"))
    })?;
    normalize_post_document(&value).map_err(|_| {
        ApiError::internal("About content storage contains an invalid document.")
    })
}

pub fn save_about_content(connection: &Connection, value: &Value) -> ApiResult<Value> {
    let body = validate_post_body(value)?;
    connection.execute(
        "INSERT INTO about_content (id, body_json) VALUES (1, ?1)\n\
         ON CONFLICT(id) DO UPDATE SET body_json = excluded.body_json",
        params![serde_json::to_string(&body)?],
    )?;
    Ok(body)
}

fn is_valid_settlement_id(value: &str) -> bool {
    !value.is_empty() && utf16_len(value) <= MAX_ID_LENGTH
}

/// True when the settlement currently belongs to a published province; gates
/// public access to the reference blocks of withdrawn settlements.
pub fn settlement_is_published(
    connection: &Connection,
    settlement_id: &str,
) -> ApiResult<bool> {
    if !is_valid_settlement_id(settlement_id) {
        return Ok(false);
    }
    let found: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM settlements s JOIN provinces p ON p.id = s.province_id\n\
             WHERE s.id = ?1 AND p.published = 1",
            params![settlement_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn get_settlement_reference(
    connection: &Connection,
    settlement_id: &str,
) -> ApiResult<Option<Value>> {
    if !is_valid_settlement_id(settlement_id) {
        return Ok(None);
    }
    let stored: Option<String> = connection
        .query_row(
            "SELECT body_json FROM settlement_references WHERE settlement_id = ?1",
            params![settlement_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&stored).map_err(|error| {
        ApiError::internal(format!(
            "Settlement reference storage contains an invalid document: {error}"
        ))
    })?;
    normalize_post_document(&value)
        .map(Some)
        .map_err(|_| {
            ApiError::internal("Settlement reference storage contains an invalid document.")
        })
}

pub fn save_settlement_reference(
    connection: &Connection,
    settlement_id: &str,
    value: &Value,
) -> ApiResult<Value> {
    if !is_valid_settlement_id(settlement_id) {
        return Err(ApiError::internal(
            "Settlement reference storage received an invalid settlement id.",
        ));
    }
    let body = validate_post_body(value)?;
    connection.execute(
        "INSERT INTO settlement_references (settlement_id, body_json) VALUES (?1, ?2)\n\
         ON CONFLICT(settlement_id) DO UPDATE SET body_json = excluded.body_json",
        params![settlement_id, serde_json::to_string(&body)?],
    )?;
    Ok(body)
}

pub fn remove_settlement_reference(
    connection: &Connection,
    settlement_id: &str,
) -> ApiResult<()> {
    if !is_valid_settlement_id(settlement_id) {
        return Ok(());
    }
    connection.execute(
        "DELETE FROM settlement_references WHERE settlement_id = ?1",
        params![settlement_id],
    )?;
    Ok(())
}

#[derive(Clone)]
pub struct ProvinceState {
    pub published: bool,
    pub slug: Option<String>,
    pub description: String,
}

fn province_state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvinceState> {
    Ok(ProvinceState {
        published: row.get::<_, i64>(1)? != 0,
        slug: row.get(2)?,
        description: row.get(3)?,
    })
}

fn load_province(connection: &Connection, id: &str) -> ApiResult<Option<ProvinceState>> {
    connection
        .query_row(
            "SELECT id, published, slug, description FROM provinces WHERE id = ?1",
            params![id],
            province_state_from_row,
        )
        .optional()
        .map_err(ApiError::from)
}

fn load_all_provinces(connection: &Connection) -> ApiResult<HashMap<String, ProvinceState>> {
    let mut statement = connection.prepare("SELECT id, published, slug, description FROM provinces")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, province_state_from_row(row)?))
    })?;
    let mut states = HashMap::new();
    for row in rows {
        let (id, state) = row?;
        states.insert(id, state);
    }
    Ok(states)
}

fn require_published(connection: &Connection, id: &str) -> ApiResult<ProvinceState> {
    let Some(state) = load_province(connection, id)? else {
        return Err(ApiError::internal(format!("province row missing for {id}")));
    };
    if !state.published {
        return Err(ApiError::not_found("Губерния не опубликована."));
    }
    Ok(state)
}

#[derive(Clone)]
pub struct SettlementRow {
    pub id: String,
    pub province_id: String,
    pub name: String,
    pub uyezd_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub created_at: String,
    pub url: Option<String>,
    pub type_name: Option<String>,
}

const SETTLEMENT_COLUMNS: &str =
    "id, province_id, name, uyezd_id, latitude, longitude, created_at, url, type";

fn settlement_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SettlementRow> {
    Ok(SettlementRow {
        id: row.get(0)?,
        province_id: row.get(1)?,
        name: row.get(2)?,
        uyezd_id: row.get(3)?,
        latitude: row.get(4)?,
        longitude: row.get(5)?,
        created_at: row.get(6)?,
        url: row.get(7)?,
        type_name: row.get(8)?,
    })
}

pub fn settlement_value(settlement: &SettlementRow) -> Value {
    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(settlement.id.clone()));
    out.insert("name".to_string(), Value::String(settlement.name.clone()));
    out.insert(
        "guberniaId".to_string(),
        Value::String(settlement.province_id.clone()),
    );
    out.insert("uyezdId".to_string(), Value::String(settlement.uyezd_id.clone()));
    out.insert(
        "latitude".to_string(),
        serde_json::json!(settlement.latitude),
    );
    out.insert(
        "longitude".to_string(),
        serde_json::json!(settlement.longitude),
    );
    out.insert(
        "createdAt".to_string(),
        Value::String(settlement.created_at.clone()),
    );
    out.insert(
        "url".to_string(),
        Value::String(effective_settlement_url(
            settlement.url.as_deref(),
            &settlement.id,
        )),
    );
    out.insert(
        "type".to_string(),
        match &settlement.type_name {
            Some(type_name) => Value::String(type_name.clone()),
            None => Value::Null,
        },
    );
    Value::Object(out)
}

fn load_settlements(connection: &Connection, province_id: &str) -> ApiResult<Vec<SettlementRow>> {
    let sql = format!(
        "SELECT {SETTLEMENT_COLUMNS} FROM settlements WHERE province_id = ?1 ORDER BY position ASC"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![province_id], settlement_from_row)?;
    let mut settlements = Vec::new();
    for row in rows {
        settlements.push(row?);
    }
    Ok(settlements)
}

fn load_settlement(
    connection: &Connection,
    province_id: &str,
    settlement_id: &str,
) -> ApiResult<Option<SettlementRow>> {
    let sql = format!(
        "SELECT {SETTLEMENT_COLUMNS} FROM settlements WHERE province_id = ?1 AND id = ?2"
    );
    connection
        .query_row(&sql, params![province_id, settlement_id], settlement_from_row)
        .optional()
        .map_err(ApiError::from)
}

#[derive(Clone)]
pub struct PostRow {
    pub id: String,
    pub province_id: String,
    pub title: String,
    pub body_json: String,
    pub created_at: String,
    pub updated_at: String,
    pub created_ms: i64,
    pub uyezd_id: Option<String>,
    pub settlement_id: Option<String>,
    pub year: String,
    pub archive_reference: String,
    pub category: Option<String>,
}

const POST_COLUMNS: &str = "id, province_id, title, body_json, created_at, updated_at, created_ms, uyezd_id, settlement_id, year, archive_reference, category";

fn post_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PostRow> {
    Ok(PostRow {
        id: row.get(0)?,
        province_id: row.get(1)?,
        title: row.get(2)?,
        body_json: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        created_ms: row.get(6)?,
        uyezd_id: row.get(7)?,
        settlement_id: row.get(8)?,
        year: row.get(9)?,
        archive_reference: row.get(10)?,
        category: row.get(11)?,
    })
}

pub fn post_value(post: &PostRow) -> ApiResult<Value> {
    let body: Value = serde_json::from_str(&post.body_json)
        .map_err(|error| ApiError::internal(format!("Stored post body is invalid JSON: {error}")))?;
    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(post.id.clone()));
    out.insert("title".to_string(), Value::String(post.title.clone()));
    out.insert("body".to_string(), body);
    out.insert(
        "createdAt".to_string(),
        Value::String(post.created_at.clone()),
    );
    out.insert(
        "updatedAt".to_string(),
        Value::String(post.updated_at.clone()),
    );
    out.insert(
        "uyezdId".to_string(),
        match &post.uyezd_id {
            Some(value) => Value::String(value.clone()),
            None => Value::Null,
        },
    );
    out.insert(
        "settlementId".to_string(),
        match &post.settlement_id {
            Some(value) => Value::String(value.clone()),
            None => Value::Null,
        },
    );
    out.insert("year".to_string(), Value::String(post.year.clone()));
    out.insert(
        "archiveReference".to_string(),
        Value::String(post.archive_reference.clone()),
    );
    out.insert(
        "category".to_string(),
        match &post.category {
            Some(value) => Value::String(value.clone()),
            None => Value::Null,
        },
    );
    Ok(Value::Object(out))
}

fn load_posts(connection: &Connection, province_id: &str) -> ApiResult<Vec<PostRow>> {
    let sql =
        format!("SELECT {POST_COLUMNS} FROM posts WHERE province_id = ?1 ORDER BY position ASC");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![province_id], post_from_row)?;
    let mut posts = Vec::new();
    for row in rows {
        posts.push(row?);
    }
    Ok(posts)
}

fn require_post(
    connection: &Connection,
    province_id: &str,
    post_id: &str,
) -> ApiResult<PostRow> {
    if post_id.is_empty() || utf16_len(post_id) > MAX_ID_LENGTH {
        return Err(ApiError::not_found("Публикация не найдена."));
    }
    let sql = format!("SELECT {POST_COLUMNS} FROM posts WHERE id = ?1 AND province_id = ?2");
    connection
        .query_row(&sql, params![post_id, province_id], post_from_row)
        .optional()?
        .ok_or_else(|| ApiError::not_found("Публикация не найдена."))
}

fn require_feature<'a>(
    canonical: &'a CanonicalProvinces,
    id: &str,
) -> ApiResult<&'a crate::geo::CanonicalFeature> {
    canonical
        .feature(id)
        .ok_or_else(|| ApiError::not_found("Губерния не найдена."))
}

pub fn build_published_gubernia(
    connection: &Connection,
    canonical: &CanonicalProvinces,
    id: &str,
    slug: &str,
    description: &str,
) -> ApiResult<Value> {
    let feature = require_feature(canonical, id)?;
    let posts = load_posts(connection, id)?
        .iter()
        .map(post_value)
        .collect::<ApiResult<Vec<_>>>()?;
    let settlements = load_settlements(connection, id)?
        .iter()
        .map(settlement_value)
        .collect::<Vec<_>>();

    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(id.to_string()));
    out.insert("name".to_string(), Value::String(feature.name.clone()));
    out.insert("slug".to_string(), Value::String(slug.to_string()));
    out.insert(
        "description".to_string(),
        Value::String(description.to_string()),
    );
    out.insert("posts".to_string(), Value::Array(posts));
    out.insert("settlements".to_string(), Value::Array(settlements));
    Ok(Value::Object(out))
}

pub fn get_gubernias_collection(
    connection: &Connection,
    canonical: &CanonicalProvinces,
) -> ApiResult<Value> {
    let states = load_all_provinces(connection)?;
    let mut features = Vec::with_capacity(canonical.features.len());
    for feature in &canonical.features {
        let Some(state) = states.get(&feature.id) else {
            return Err(ApiError::internal(format!(
                "province row missing for {}",
                feature.id
            )));
        };
        features.push(CanonicalProvinces::merged_feature(
            feature,
            state.published,
            state.slug.as_deref(),
        ));
    }
    Ok(serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    }))
}

pub fn get_publication_metrics(
    connection: &Connection,
    canonical: &CanonicalProvinces,
) -> ApiResult<Value> {
    let states = load_all_provinces(connection)?;

    let mut post_counts: HashMap<String, i64> = HashMap::new();
    {
        let mut statement =
            connection.prepare("SELECT province_id, COUNT(*) FROM posts GROUP BY province_id")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (id, count) = row?;
            post_counts.insert(id, count);
        }
    }
    let mut settlement_counts: HashMap<String, i64> = HashMap::new();
    {
        let mut statement = connection
            .prepare("SELECT province_id, COUNT(*) FROM settlements GROUP BY province_id")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (id, count) = row?;
            settlement_counts.insert(id, count);
        }
    }

    let mut total_posts: i64 = 0;
    let mut total_settlements: i64 = 0;
    let mut provinces: Vec<Value> = Vec::new();
    for feature in &canonical.features {
        let Some(state) = states.get(&feature.id) else {
            return Err(ApiError::internal(format!(
                "province row missing for {}",
                feature.id
            )));
        };
        if !state.published {
            continue;
        }
        let posts_count = *post_counts.get(&feature.id).unwrap_or(&0);
        let settlements_count = *settlement_counts.get(&feature.id).unwrap_or(&0);
        total_posts += posts_count;
        total_settlements += settlements_count;
        provinces.push(serde_json::json!({
            "id": feature.id,
            "name": feature.name,
            "slug": state.slug.clone().unwrap_or_default(),
            "postsCount": posts_count,
            "settlementsCount": settlements_count,
        }));
    }

    provinces.sort_by(|left, right| {
        let left_name = left["name"].as_str().unwrap_or_default();
        let right_name = right["name"].as_str().unwrap_or_default();
        ru_compare(left_name, right_name).then_with(|| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        })
    });

    Ok(serde_json::json!({
        "totalHistoricalGubernias": canonical.features.len(),
        "publishedCount": provinces.len(),
        "totalPublishedPosts": total_posts,
        "totalPublishedSettlements": total_settlements,
        "provinces": provinces,
    }))
}

pub fn get_published_geo_data(
    connection: &Connection,
    canonical: &CanonicalProvinces,
) -> ApiResult<Value> {
    let states = load_all_provinces(connection)?;
    let mut provinces: Vec<Value> = Vec::new();
    let mut settlements: Vec<Value> = Vec::new();
    for feature in &canonical.features {
        let Some(state) = states.get(&feature.id) else {
            return Err(ApiError::internal(format!(
                "province row missing for {}",
                feature.id
            )));
        };
        if !state.published {
            continue;
        }
        provinces.push(serde_json::json!({
            "id": feature.id,
            "name": feature.name,
            "slug": state.slug.clone().unwrap_or_default(),
        }));
        for settlement in load_settlements(connection, &feature.id)? {
            settlements.push(settlement_value(&settlement));
        }
    }
    Ok(serde_json::json!({
        "provinces": provinces,
        "settlements": settlements,
    }))
}

pub fn find_published_gubernia_by_slug(
    connection: &Connection,
    canonical: &CanonicalProvinces,
    slug: &str,
) -> ApiResult<Option<Value>> {
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT id, slug, description FROM provinces WHERE published = 1 AND slug = ?1",
            params![slug],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match row {
        Some((id, slug, description)) => {
            Ok(Some(build_published_gubernia(connection, canonical, &id, &slug, &description)?))
        }
        None => Ok(None),
    }
}

pub fn find_published_settlement_id_by_slug(
    connection: &Connection,
    slug: &str,
) -> ApiResult<Option<String>> {
    if !is_valid_slug(slug) {
        return Ok(None);
    }
    let url = format!("{SETTLEMENT_URL_PREFIX}{slug}");
    connection
        .query_row(
            "SELECT s.id FROM settlements s JOIN provinces p ON p.id = s.province_id\n\
             WHERE p.published = 1 AND COALESCE(s.url, '/naselennyy-punkt/' || s.id) = ?1\n\
             LIMIT 1",
            params![url],
            |row| row.get(0),
        )
        .optional()
        .map_err(ApiError::from)
}

pub fn find_published_settlement_by_slug(
    connection: &Connection,
    canonical: &CanonicalProvinces,
    slug: &str,
) -> ApiResult<Option<Value>> {
    if !is_valid_slug(slug) {
        return Ok(None);
    }
    let url = format!("{SETTLEMENT_URL_PREFIX}{slug}");
    let sql = "SELECT s.id, s.province_id, s.name, s.uyezd_id, s.latitude, s.longitude, s.created_at, s.url, s.type, p.slug, p.description\n\
         FROM settlements s JOIN provinces p ON p.id = s.province_id\n\
         WHERE p.published = 1 AND COALESCE(s.url, '/naselennyy-punkt/' || s.id) = ?1\n\
         LIMIT 1";
    let row: Option<(SettlementRow, String, String)> = connection
        .query_row(&sql, params![url], |row| {
            Ok((settlement_from_row(row)?, row.get(9)?, row.get(10)?))
        })
        .optional()?;
    let Some((settlement, slug, description)) = row else {
        return Ok(None);
    };
    let gubernia = build_published_gubernia(
        connection,
        canonical,
        &settlement.province_id,
        &slug,
        &description,
    )?;
    Ok(Some(serde_json::json!({
        "settlement": settlement_value(&settlement),
        "gubernia": gubernia,
    })))
}

pub fn publish_gubernia(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    id: &str,
    slug_value: &Value,
) -> ApiResult<Value> {
    let slug = validate_slug(slug_value)?;
    require_feature(canonical, id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let state = load_province(&transaction, id)?
        .ok_or_else(|| ApiError::internal(format!("province row missing for {id}")))?;
    if state.published {
        return Err(ApiError::conflict("Губерния уже опубликована."));
    }
    assert_unique_slug(&transaction, &slug, Some(id))?;
    transaction.execute(
        "UPDATE provinces SET published = 1, slug = ?1, description = '' WHERE id = ?2",
        params![slug, id],
    )?;
    let gubernia = build_published_gubernia(&transaction, canonical, id, &slug, "")?;
    transaction.commit()?;
    Ok(gubernia)
}

pub fn update_gubernia_publication(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    id: &str,
    slug_value: &Value,
    description_value: &Value,
) -> ApiResult<Value> {
    let slug = validate_slug(slug_value)?;
    let description = validate_description(description_value)?;
    require_feature(canonical, id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, id)?;
    assert_unique_slug(&transaction, &slug, Some(id))?;
    transaction.execute(
        "UPDATE provinces SET slug = ?1, description = ?2 WHERE id = ?3",
        params![slug, description, id],
    )?;
    let gubernia = build_published_gubernia(&transaction, canonical, id, &slug, &description)?;
    transaction.commit()?;
    Ok(gubernia)
}

fn assert_unique_slug(
    connection: &Connection,
    slug: &str,
    except_id: Option<&str>,
) -> ApiResult<()> {
    let collision: Option<String> = connection
        .query_row(
            "SELECT id FROM provinces WHERE slug = ?1 AND (?2 IS NULL OR id <> ?2)",
            params![slug, except_id],
            |row| row.get(0),
        )
        .optional()?;
    if collision.is_some() {
        return Err(ApiError::conflict(
            "Этот slug уже используется другой губернией.",
        ));
    }
    Ok(())
}

pub fn unpublish_gubernia(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    id: &str,
) -> ApiResult<()> {
    require_feature(canonical, id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, id)?;
    transaction.execute("DELETE FROM posts WHERE province_id = ?1", params![id])?;
    transaction.execute("DELETE FROM settlements WHERE province_id = ?1", params![id])?;
    transaction.execute(
        "UPDATE provinces SET published = 0, slug = NULL, description = '' WHERE id = ?1",
        params![id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn assert_unique_settlement_url(connection: &Connection, url: &str) -> ApiResult<()> {
    let mut statement = connection.prepare("SELECT id, url FROM settlements")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (id, stored) = row?;
        if effective_settlement_url(stored.as_deref(), &id) == url {
            return Err(ApiError::conflict(
                "Этот адрес страницы уже используется другим населённым пунктом.",
            ));
        }
    }
    Ok(())
}

/// District identity comes from the canonical district files but is enforced
/// from SQLite: writes may only reference an existing (province, district) pair.
fn district_exists(connection: &Connection, province_id: &str, uyezd_id: &str) -> ApiResult<bool> {
    let found: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM districts WHERE id = ?1 AND province_id = ?2",
            params![uyezd_id, province_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn resolve_post_placement(
    connection: &Connection,
    province_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<(Option<String>, Option<String>)> {
    let settlement_id =
        normalize_optional_reference_id(payload.get("settlementId"), "Населённый пункт")?;
    let requested_uyezd = normalize_optional_reference_id(payload.get("uyezdId"), "Уезд")?;

    if let Some(settlement_id) = settlement_id {
        let Some(settlement) = load_settlement(connection, province_id, &settlement_id)? else {
            return Err(bad("Населённый пункт не найден в этой губернии."));
        };
        if let Some(requested_uyezd) = requested_uyezd {
            if requested_uyezd != settlement.uyezd_id {
                return Err(bad("Уезд не соответствует выбранному населённому пункту."));
            }
        }
        return Ok((Some(settlement.uyezd_id), Some(settlement_id)));
    }

    if let Some(requested_uyezd) = requested_uyezd.as_deref() {
        if !district_exists(connection, province_id, requested_uyezd)? {
            return Err(bad("Уезд не найден в этой губернии."));
        }
    }
    Ok((requested_uyezd, None))
}

pub fn create_post(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    gubernia_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<Value> {
    let title = validate_post_title(payload.get("title").unwrap_or(&Value::Null))?;
    let body = validate_post_body(payload.get("body").unwrap_or(&Value::Null))?;
    require_feature(canonical, gubernia_id)?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    let (uyezd_id, settlement_id) = resolve_post_placement(&transaction, gubernia_id, payload)?;
    let settings = load_settings_lists(&transaction)?;
    let category = validate_post_category(
        payload.get("category").unwrap_or(&Value::Null),
        &settings.categories,
    )?;
    let year = normalize_optional_text(payload.get("year"), "Год", MAX_YEAR_LENGTH)?;
    let archive_reference = normalize_optional_text(
        payload.get("archiveReference"),
        "Архивный шифр",
        MAX_ARCHIVE_REFERENCE_LENGTH,
    )?;

    let timestamp = iso_now();
    let post = PostRow {
        id: random_id(),
        province_id: gubernia_id.to_string(),
        title,
        body_json: serde_json::to_string(&body)?,
        created_at: timestamp.clone(),
        updated_at: timestamp.clone(),
        created_ms: parse_js_date_ms(&timestamp).unwrap_or_default(),
        uyezd_id,
        settlement_id,
        year,
        archive_reference,
        category: Some(category),
    };
    let position: i64 = transaction.query_row(
        "SELECT COALESCE(MIN(position), 0) - 1 FROM posts WHERE province_id = ?1",
        params![gubernia_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO posts (id, province_id, title, body_json, created_at, updated_at, created_ms, position, uyezd_id, settlement_id, year, archive_reference, category)\n\
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            post.id,
            post.province_id,
            post.title,
            post.body_json,
            post.created_at,
            post.updated_at,
            post.created_ms,
            position,
            post.uyezd_id,
            post.settlement_id,
            post.year,
            post.archive_reference,
            post.category,
        ],
    )?;
    let value = post_value(&post)?;
    transaction.commit()?;
    Ok(value)
}

pub fn update_post(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    gubernia_id: &str,
    post_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<Value> {
    let title = validate_post_title(payload.get("title").unwrap_or(&Value::Null))?;
    let body = validate_post_body(payload.get("body").unwrap_or(&Value::Null))?;
    require_feature(canonical, gubernia_id)?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    let post = require_post(&transaction, gubernia_id, post_id)?;

    let target_gubernia_id =
        normalize_optional_reference_id(payload.get("targetGuberniaId"), "Губерния")?
            .unwrap_or_else(|| gubernia_id.to_string());
    require_feature(canonical, &target_gubernia_id)?;
    require_published(&transaction, &target_gubernia_id)?;
    let (uyezd_id, settlement_id) =
        resolve_post_placement(&transaction, &target_gubernia_id, payload)?;
    let settings = load_settings_lists(&transaction)?;
    // An unchanged category is a historical snapshot: it stays valid even if
    // the settings option was renamed or removed, but any other value must be
    // one of the current options.
    let category = match (post.category.as_deref(), payload.get("category")) {
        (Some(stored), Some(Value::String(current))) if current == stored => {
            Some(stored.to_string())
        }
        _ => Some(validate_post_category(
            payload.get("category").unwrap_or(&Value::Null),
            &settings.categories,
        )?),
    };
    let year = normalize_optional_text(payload.get("year"), "Год", MAX_YEAR_LENGTH)?;
    let archive_reference = normalize_optional_text(
        payload.get("archiveReference"),
        "Архивный шифр",
        MAX_ARCHIVE_REFERENCE_LENGTH,
    )?;

    let updated = PostRow {
        id: post.id.clone(),
        province_id: target_gubernia_id.clone(),
        title,
        body_json: serde_json::to_string(&body)?,
        created_at: post.created_at.clone(),
        updated_at: iso_now(),
        created_ms: post.created_ms,
        uyezd_id,
        settlement_id,
        year,
        archive_reference,
        category,
    };

    if target_gubernia_id == gubernia_id {
        transaction.execute(
            "UPDATE posts SET title = ?1, body_json = ?2, uyezd_id = ?3, settlement_id = ?4,\n\
             year = ?5, archive_reference = ?6, category = ?7, updated_at = ?8\n\
             WHERE id = ?9 AND province_id = ?10",
            params![
                updated.title,
                updated.body_json,
                updated.uyezd_id,
                updated.settlement_id,
                updated.year,
                updated.archive_reference,
                updated.category,
                updated.updated_at,
                updated.id,
                gubernia_id,
            ],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM posts WHERE id = ?1 AND province_id = ?2",
            params![post.id, gubernia_id],
        )?;
        // Keeps the destination list newest-first: the moved post keeps its
        // original createdAt and lands before older posts but after equal ones.
        let insert_position: Option<i64> = transaction
            .query_row(
                "SELECT position FROM posts WHERE province_id = ?1 AND created_ms < ?2\n\
                 ORDER BY position ASC LIMIT 1",
                params![target_gubernia_id, post.created_ms],
                |row| row.get(0),
            )
            .optional()?;
        let position = match insert_position {
            Some(position) => {
                transaction.execute(
                    "UPDATE posts SET position = position + 1 WHERE province_id = ?1 AND position >= ?2",
                    params![target_gubernia_id, position],
                )?;
                position
            }
            None => transaction.query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM posts WHERE province_id = ?1",
                params![target_gubernia_id],
                |row| row.get(0),
            )?,
        };
        transaction.execute(
            "INSERT INTO posts (id, province_id, title, body_json, created_at, updated_at, created_ms, position, uyezd_id, settlement_id, year, archive_reference, category)\n\
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                updated.id,
                updated.province_id,
                updated.title,
                updated.body_json,
                updated.created_at,
                updated.updated_at,
                updated.created_ms,
                position,
                updated.uyezd_id,
                updated.settlement_id,
                updated.year,
                updated.archive_reference,
                updated.category,
            ],
        )?;
    }
    let value = post_value(&updated)?;
    transaction.commit()?;
    Ok(value)
}

pub fn delete_post(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    gubernia_id: &str,
    post_id: &str,
) -> ApiResult<()> {
    require_feature(canonical, gubernia_id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    require_post(&transaction, gubernia_id, post_id)?;
    transaction.execute(
        "DELETE FROM posts WHERE id = ?1 AND province_id = ?2",
        params![post_id, gubernia_id],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn create_settlement(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
    gubernia_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<Value> {
    let name = validate_settlement_name(payload.get("name").unwrap_or(&Value::Null))?;
    let url = validate_settlement_url(payload.get("url").unwrap_or(&Value::Null))?;
    let settings = load_settings_lists(connection)?;
    let type_name = validate_settlement_type(
        payload.get("type").unwrap_or(&Value::Null),
        &settings.settlement_types,
    )?;
    let Some(uyezd_id) = normalize_optional_reference_id(payload.get("uyezdId"), "Уезд")? else {
        return Err(bad("Уезд обязателен для населённого пункта."));
    };
    let (latitude, longitude) = parse_coordinates(payload.get("coordinates").unwrap_or(&Value::Null))?;
    require_feature(canonical, gubernia_id)?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    if !district_exists(&transaction, gubernia_id, &uyezd_id)? {
        return Err(bad("Уезд не найден в этой губернии."));
    }
    // The admin types "latitude, longitude"; the point-in-district test maps it
    // to GeoJSON's [longitude, latitude] order, hole boundaries included.
    if !geo
        .is_point_in_uyezd(gubernia_id, &uyezd_id, longitude, latitude)
        .map_err(ApiError::internal)?
    {
        return Err(bad("Точка находится за пределами выбранного уезда."));
    }
    assert_unique_settlement_url(&transaction, &url)?;

    let settlement = SettlementRow {
        id: random_id(),
        province_id: gubernia_id.to_string(),
        name,
        uyezd_id,
        latitude,
        longitude,
        created_at: iso_now(),
        url: Some(url),
        type_name: Some(type_name),
    };
    let position: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM settlements WHERE province_id = ?1",
        params![gubernia_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO settlements (id, province_id, name, uyezd_id, latitude, longitude, created_at, position, url, type)\n\
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            settlement.id,
            settlement.province_id,
            settlement.name,
            settlement.uyezd_id,
            settlement.latitude,
            settlement.longitude,
            settlement.created_at,
            position,
            settlement.url,
            settlement.type_name,
        ],
    )?;
    let value = settlement_value(&settlement);
    transaction.commit()?;
    Ok(value)
}

pub fn delete_settlement(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    gubernia_id: &str,
    settlement_id: &str,
) -> ApiResult<()> {
    require_feature(canonical, gubernia_id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    if settlement_id.is_empty()
        || utf16_len(settlement_id) > MAX_ID_LENGTH
        || load_settlement(&transaction, gubernia_id, settlement_id)?.is_none()
    {
        return Err(ApiError::not_found("Населённый пункт не найден."));
    }
    transaction.execute(
        "DELETE FROM settlements WHERE id = ?1 AND province_id = ?2",
        params![settlement_id, gubernia_id],
    )?;
    transaction.execute(
        "UPDATE posts SET settlement_id = NULL WHERE province_id = ?1 AND settlement_id = ?2",
        params![gubernia_id, settlement_id],
    )?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn titles_and_names_are_trimmed_and_bounded() {
        assert_eq!(validate_post_title(&json!("  тест  ")).unwrap(), "тест");
        assert_eq!(
            validate_post_title(&json!("   ")).unwrap_err().message,
            "Заголовок не должен быть пустым."
        );
        assert_eq!(
            validate_post_title(&json!(5)).unwrap_err().message,
            "Заголовок должен быть строкой."
        );
        assert_eq!(
            validate_settlement_name(&json!("Павелец")).unwrap(),
            "Павелец"
        );
        assert_eq!(
            validate_settlement_name(&json!("")).unwrap_err().message,
            "Название населённого пункта не должно быть пустым."
        );
    }

    #[test]
    fn coordinates_follow_the_admin_string_format() {
        assert_eq!(parse_coordinates(&json!("53.8, 39.2")).unwrap(), (53.8, 39.2));
        assert_eq!(
            parse_coordinates(&json!("1,2,3")).unwrap_err().message,
            "Координаты должны содержать широту и долготу через запятую."
        );
        assert_eq!(
            parse_coordinates(&json!("91, 0")).unwrap_err().message,
            "Координаты выходят за допустимые пределы."
        );
        assert_eq!(
            parse_coordinates(&json!(", 10")).unwrap_err().message,
            "Широта и долгота должны быть числами."
        );
    }

    #[test]
    fn category_messages_list_current_options() {
        let allowed = vec!["Статья".to_string(), "Персона".to_string()];
        assert_eq!(
            validate_post_category(&json!("Персона"), &allowed).unwrap(),
            "Персона"
        );
        assert_eq!(
            validate_post_category(&json!("Прочее"), &allowed)
                .unwrap_err()
                .message,
            "Категория должна быть одной из: «Статья», «Персона»."
        );
    }

    #[test]
    fn optional_helpers_mirror_undefined_and_null() {
        assert_eq!(normalize_optional_reference_id(None, "Уезд").unwrap(), None);
        assert_eq!(
            normalize_optional_reference_id(Some(&json!("")), "Уезд").unwrap(),
            None
        );
        assert_eq!(
            normalize_optional_text(Some(&json!(" 1900 ")), "Год", 100).unwrap(),
            "1900"
        );
        assert_eq!(
            normalize_optional_text(Some(&json!(1900)), "Год", 100)
                .unwrap_err()
                .message,
            "Поле «Год» должно быть строкой."
        );
    }

    #[test]
    fn settings_names_collapse_whitespace() {
        assert_eq!(
            normalize_setting_name(&json!("  Новое   название ")).unwrap(),
            "Новое название"
        );
        assert_eq!(
            normalize_setting_name(&json!("   ")).unwrap_err(),
            "Введите название."
        );
        assert_eq!(
            normalize_setting_name(&json!(42)).unwrap_err(),
            "Введите название текстом."
        );
    }
}
