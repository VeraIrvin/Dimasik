use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value};

use crate::content::{
    normalize_optional_document, normalize_post_document, plain_text_to_document, ContentError,
    EMPTY_DOCUMENT_MESSAGE,
};
use crate::error::{ApiError, ApiResult};
use crate::geo::{CanonicalProvinces, GeoRuntime};
use crate::util::{
    effective_settlement_url, is_valid_settlement_url, is_valid_slug, iso_now, js_collapse_whitespace,
    js_number, js_trim, now_unix, parse_js_date_ms, random_id, ru_compare, utf16_len,
    SETTLEMENT_URL_PREFIX,
};

pub const MAX_TITLE_LENGTH: usize = 1_000;
pub const MAX_ID_LENGTH: usize = 100;
pub const MAX_SETTLEMENT_NAME_LENGTH: usize = 200;
pub const MAX_YEAR_LENGTH: usize = 100;
pub const MAX_ARCHIVE_REFERENCE_LENGTH: usize = 300;
/// At most ten uploaded images may be attached to one publication.
pub const MAX_POST_IMAGES: usize = 10;
/// Pending uploads older than this are withdrawn from claiming and deleted by
/// the cleanup sweep (storage objects first, row last).
pub const PENDING_IMAGE_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
/// A provisional upload row (written before the storage PUTs) is only treated
/// as abandoned after this grace period, so a slow upload is never swept
/// while its objects are still being written.
pub const IMAGE_UPLOAD_GRACE_MS: i64 = 30 * 60 * 1_000;
/// `attached_ms` marker of a row the cleanup sweep has fenced as its own: the
/// pass has claimed the row's objects, so an in-flight upload can no longer
/// complete it, no post can claim it and the user cannot cancel it. Only
/// [`finish_image_cleanup`], after both objects are confirmed gone, drops it.
const IMAGE_CLEANUP_FENCED_MS: i64 = -1;
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

const INVALID_DESCRIPTION_MESSAGE: &str = "Некорректное содержимое описания.";

/// Validates the province `description` payload: the field must be present and
/// hold a rich document. `null` (or a blank document) clears it, while every
/// other value must pass the post whitelist and resource bounds.
pub fn validate_description(value: Option<&Value>) -> ApiResult<Option<Value>> {
    let Some(value) = value else {
        return Err(bad(INVALID_DESCRIPTION_MESSAGE));
    };
    normalize_optional_document(value).map_err(|_| bad(INVALID_DESCRIPTION_MESSAGE))
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

/// Strictly validates a submitted post body, except that an actually empty
/// document is reported as `None` so the caller can substitute the canonical
/// empty document when images are present; `null` and every structural
/// problem keep the strict rejection and its precedence.
fn validate_optional_post_body(value: &Value) -> ApiResult<Option<Value>> {
    if value.is_null() {
        return validate_post_body(value).map(Some);
    }
    normalize_optional_document(value).map_err(|ContentError(message)| bad(message))
}

/// Turns a body from [`validate_optional_post_body`] into its stored form:
/// only a record that has (or claims) images may store the canonical empty
/// document, everything else keeps the strict empty-body rejection.
fn resolve_post_body(body: Option<Value>, has_images: bool) -> ApiResult<Value> {
    match body {
        Some(body) => Ok(body),
        None if has_images => Ok(plain_text_to_document("")),
        None => Err(bad(EMPTY_DOCUMENT_MESSAGE)),
    }
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
}

fn province_state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvinceState> {
    Ok(ProvinceState {
        published: row.get::<_, i64>(1)? != 0,
        slug: row.get(2)?,
    })
}

fn load_province(connection: &Connection, id: &str) -> ApiResult<Option<ProvinceState>> {
    connection
        .query_row(
            "SELECT id, published, slug FROM provinces WHERE id = ?1",
            params![id],
            province_state_from_row,
        )
        .optional()
        .map_err(ApiError::from)
}

fn load_all_provinces(connection: &Connection) -> ApiResult<HashMap<String, ProvinceState>> {
    let mut statement = connection.prepare("SELECT id, published, slug FROM provinces")?;
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
pub struct PostImageRow {
    pub id: String,
    pub post_id: Option<String>,
    pub original_key: String,
    pub thumbnail_key: String,
    pub width: i64,
    pub height: i64,
    pub position: i64,
    pub created_at: String,
    pub created_ms: i64,
    /// `NULL` = upload completed, never attached (claimable while fresh);
    /// `0` = provisional row written before the storage PUTs; a negative
    /// marker = the cleanup sweep has fenced the row (it can no longer be
    /// completed, claimed or cancelled); a positive timestamp = claimed-at
    /// and, once `post_id` is NULL again, the durable marker that the storage
    /// objects still need deleting.
    pub attached_ms: Option<i64>,
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
    /// Attached images in stored order; filled by the loaders and by
    /// `create_post`/`update_post`.
    pub images: Vec<PostImageRow>,
}

const POST_COLUMNS: &str = "id, province_id, title, body_json, created_at, updated_at, created_ms, uyezd_id, settlement_id, year, archive_reference, category";
const IMAGE_COLUMNS: &str = "id, post_id, original_key, thumbnail_key, width, height, position, created_at, created_ms, attached_ms";
const IMAGE_COLUMNS_I: &str = "i.id, i.post_id, i.original_key, i.thumbnail_key, i.width, i.height, i.position, i.created_at, i.created_ms, i.attached_ms";

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
        images: Vec::new(),
    })
}

fn image_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PostImageRow> {
    Ok(PostImageRow {
        id: row.get(0)?,
        post_id: row.get(1)?,
        original_key: row.get(2)?,
        thumbnail_key: row.get(3)?,
        width: row.get(4)?,
        height: row.get(5)?,
        position: row.get(6)?,
        created_at: row.get(7)?,
        created_ms: row.get(8)?,
        attached_ms: row.get(9)?,
    })
}

/// The public DTO of one image: same-origin URLs only, never storage keys.
pub fn image_value(image: &PostImageRow) -> Value {
    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(image.id.clone()));
    out.insert("width".to_string(), Value::from(image.width));
    out.insert("height".to_string(), Value::from(image.height));
    out.insert(
        "originalUrl".to_string(),
        Value::String(image_endpoint(&image.id, "original")),
    );
    out.insert(
        "thumbnailUrl".to_string(),
        Value::String(image_endpoint(&image.id, "thumbnail")),
    );
    Value::Object(out)
}

fn image_endpoint(id: &str, variant: &str) -> String {
    format!("/api/post-images/{id}/{variant}")
}

fn load_post_images(connection: &Connection, post_id: &str) -> ApiResult<Vec<PostImageRow>> {
    let sql =
        format!("SELECT {IMAGE_COLUMNS} FROM post_images WHERE post_id = ?1 ORDER BY position ASC");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![post_id], image_from_row)?;
    let mut images = Vec::new();
    for row in rows {
        images.push(row?);
    }
    Ok(images)
}

/// One query for a whole province: the feed serializes every post together
/// with its images, so it must not issue a query per post (N+1).
fn load_province_post_images(
    connection: &Connection,
    province_id: &str,
) -> ApiResult<HashMap<String, Vec<PostImageRow>>> {
    let sql = format!(
        "SELECT {IMAGE_COLUMNS_I} FROM post_images i\n\
         JOIN posts p ON p.id = i.post_id\n\
         WHERE p.province_id = ?1 ORDER BY i.position ASC"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![province_id], image_from_row)?;
    let mut grouped: HashMap<String, Vec<PostImageRow>> = HashMap::new();
    for row in rows {
        let image = row?;
        if let Some(post_id) = image.post_id.clone() {
            grouped.entry(post_id).or_default().push(image);
        }
    }
    Ok(grouped)
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
    // Every serialized post carries the images array (possibly empty), so the
    // public contract never has to distinguish "absent" from "none".
    out.insert(
        "images".to_string(),
        Value::Array(post.images.iter().map(image_value).collect()),
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
    let mut images_by_post = load_province_post_images(connection, province_id)?;
    for post in &mut posts {
        if let Some(images) = images_by_post.remove(&post.id) {
            post.images = images;
        }
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
    let mut post = connection
        .query_row(&sql, params![post_id, province_id], post_from_row)
        .optional()?
        .ok_or_else(|| ApiError::not_found("Публикация не найдена."))?;
    post.images = load_post_images(connection, &post.id)?;
    Ok(post)
}

fn require_feature<'a>(
    canonical: &'a CanonicalProvinces,
    id: &str,
) -> ApiResult<&'a crate::geo::CanonicalFeature> {
    canonical
        .feature(id)
        .ok_or_else(|| ApiError::not_found("Губерния не найдена."))
}

// ---------------------------------------------------------------------------
// Post images: upload metadata, claiming, reads and object cleanup
// ---------------------------------------------------------------------------

/// Validates one submitted `imageIds` list: an array of at most ten unique
/// upload ids. The order is preserved because it becomes the stored image
/// order; URLs and every other value are rejected.
fn validate_image_id_list(value: &Value) -> ApiResult<Vec<String>> {
    let Some(items) = value.as_array() else {
        return Err(bad("Список изображений должен быть массивом."));
    };
    if items.len() > MAX_POST_IMAGES {
        return Err(bad(format!(
            "К публикации можно приложить не более {MAX_POST_IMAGES} изображений."
        )));
    }
    let mut ids: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item.as_str() else {
            return Err(bad("Некорректный идентификатор изображения."));
        };
        let id = js_trim(id);
        if id.is_empty() || utf16_len(id) > MAX_ID_LENGTH {
            return Err(bad("Некорректный идентификатор изображения."));
        }
        if ids.iter().any(|existing| existing == id) {
            return Err(bad("Идентификаторы изображений не должны повторяться."));
        }
        ids.push(id.to_string());
    }
    Ok(ids)
}

/// Creation historically treats an absent or null `imageIds` field as no
/// attachments.
pub fn validate_image_ids(payload: &Map<String, Value>) -> ApiResult<Vec<String>> {
    optional(payload.get("imageIds"))
        .map(validate_image_id_list)
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// Validates the optional `imageIds` field of a post PATCH: a present array
/// is a full ordered replacement (an explicitly empty list removes every
/// image) while an absent field keeps the stored list. Creation tolerates
/// `null` as "no attachments"; for a replacement `null` is not a list and is
/// rejected instead of guessing between "keep" and "remove every image".
fn validate_image_ids_replacement(
    payload: &Map<String, Value>,
) -> ApiResult<Option<Vec<String>>> {
    payload
        .get("imageIds")
        .map(validate_image_id_list)
        .transpose()
}

/// Claims one pending upload for a post inside the caller's transaction: the
/// row must be fresh, never-attached and unexpired, and `position` becomes
/// its stored order. A lost compare-and-set is classified by
/// [`claim_failure`], so an id that is unknown, expired, fenced for cleanup
/// or already attached to another post never becomes a silent no-op.
fn claim_post_image(
    transaction: &rusqlite::Transaction<'_>,
    post_id: &str,
    image_id: &str,
    position: i64,
    now_ms: i64,
) -> ApiResult<()> {
    let cutoff_ms = now_ms - PENDING_IMAGE_TTL_MS;
    let updated = transaction.execute(
        "UPDATE post_images SET post_id = ?1, position = ?2, attached_ms = ?3\n\
         WHERE id = ?4 AND post_id IS NULL AND attached_ms IS NULL AND created_ms >= ?5",
        params![post_id, position, now_ms, image_id, cutoff_ms],
    )?;
    if updated == 0 {
        return Err(claim_failure(transaction, image_id)?);
    }
    Ok(())
}

/// Claims pending uploads for a just-inserted post inside the caller's
/// transaction: every id must be a fresh, never-attached pending row, and the
/// submitted order becomes `position`. Any failure aborts the whole creation,
/// so a post can never be half-populated.
fn claim_post_images(
    transaction: &rusqlite::Transaction<'_>,
    post_id: &str,
    image_ids: &[String],
    now_ms: i64,
) -> ApiResult<()> {
    for (position, image_id) in image_ids.iter().enumerate() {
        claim_post_image(transaction, post_id, image_id, position as i64, now_ms)?;
    }
    Ok(())
}

/// Applies the PATCH `imageIds` full ordered replacement inside the caller's
/// transaction. Every submitted id either already belongs to this post
/// (retained and re-ordered) or must be a fresh pending upload claimed
/// through the same compare-and-set as creation; every stored image that is
/// no longer listed is detached with the durable cleanup marker (`post_id`
/// NULL, `attached_ms` stamped now, object keys kept). Any failure aborts the
/// whole save, so the attachment set, the stored order and the metadata
/// always change together and a failed request never leaves a detached row
/// behind.
///
/// Returns whether any image was detached, i.e. whether the storage sweep
/// has to run once the transaction has committed.
fn replace_post_images(
    transaction: &rusqlite::Transaction<'_>,
    post_id: &str,
    image_ids: &[String],
    current: &[PostImageRow],
    now_ms: i64,
) -> ApiResult<bool> {
    for (position, image_id) in image_ids.iter().enumerate() {
        if current.iter().any(|image| &image.id == image_id) {
            transaction.execute(
                "UPDATE post_images SET position = ?1 WHERE id = ?2 AND post_id = ?3",
                params![position as i64, image_id, post_id],
            )?;
        } else {
            claim_post_image(transaction, post_id, image_id, position as i64, now_ms)?;
        }
    }
    let mut detached = false;
    for image in current {
        if image_ids.contains(&image.id) {
            continue;
        }
        // The marker is written in the same transaction as the save: objects
        // are only ever scheduled for deletion by committed state, and the
        // sweep deletes them before dropping the row.
        let updated = transaction.execute(
            "UPDATE post_images SET post_id = NULL, attached_ms = ?1\n\
             WHERE id = ?2 AND post_id = ?3 AND attached_ms > 0",
            params![now_ms, image.id, post_id],
        )?;
        if updated == 0 {
            return Err(ApiError::internal(format!(
                "image {} lost its attachment while replacing images of post {post_id}",
                image.id
            )));
        }
        detached = true;
    }
    Ok(detached)
}

/// Classifies a claim that lost its compare-and-set — attaching a pending
/// upload to a post or completing a provisional one — so the API can answer
/// 404 (missing, expired or already fenced for cleanup) or 409 (already
/// attached) instead of a generic 400 or 500.
fn claim_failure(
    connection: &Connection,
    image_id: &str,
) -> ApiResult<ApiError> {
    let existing: Option<(Option<String>, Option<i64>)> = connection
        .query_row(
            "SELECT post_id, attached_ms FROM post_images WHERE id = ?1",
            params![image_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match existing {
        Some((Some(_), _)) => ApiError::conflict("Изображение уже привязано к публикации."),
        Some((None, Some(_))) => {
            ApiError::not_found("Изображение не найдено: загрузка удалена.")
        }
        Some((None, None)) => ApiError::not_found("Срок загрузки изображения истёк."),
        None => ApiError::not_found("Изображение не найдено."),
    })
}

/// A stored image plus whether its province is currently published; the read
/// route uses this to decide between admin-only and public access.
pub struct ImageAccess {
    pub image: PostImageRow,
    pub published: bool,
}

/// `true` for a pending upload that may still be shown to its uploader and
/// claimed by a new post; expired or cancelled rows are only cleanup matter.
pub fn is_fresh_pending(image: &PostImageRow, now_ms: i64) -> bool {
    image.post_id.is_none()
        && image.attached_ms.is_none()
        && image.created_ms >= now_ms - PENDING_IMAGE_TTL_MS
}

pub fn load_image_access(connection: &Connection, id: &str) -> ApiResult<Option<ImageAccess>> {
    if id.is_empty() || utf16_len(id) > MAX_ID_LENGTH {
        return Ok(None);
    }
    let sql = "SELECT i.id, i.post_id, i.original_key, i.thumbnail_key, i.width, i.height,\n\
                      i.position, i.created_at, i.created_ms, i.attached_ms,\n\
                      CASE WHEN p.published = 1 THEN 1 ELSE 0 END\n\
               FROM post_images i\n\
               LEFT JOIN posts po ON po.id = i.post_id\n\
               LEFT JOIN provinces p ON p.id = po.province_id\n\
               WHERE i.id = ?1";
    let access = connection
        .query_row(sql, params![id], |row| {
            Ok(ImageAccess {
                image: image_from_row(row)?,
                published: row.get::<_, i64>(10)? == 1,
            })
        })
        .optional()?;
    Ok(access)
}

/// Writes the provisional cleanup record before any storage call:
/// `attached_ms = 0`, zero dimensions, deterministic keys. Such a row is not
/// claimable and is only touched by the cleanup sweep after the upload grace.
pub fn begin_image_upload(connection: &Connection, id: &str) -> ApiResult<()> {
    let timestamp = iso_now();
    let created_ms = parse_js_date_ms(&timestamp).unwrap_or_else(|| now_unix() * 1_000);
    let (original_key, thumbnail_key) = crate::s3::image_keys(id)?;
    connection.execute(
        "INSERT INTO post_images (id, post_id, original_key, thumbnail_key, width, height, position, created_at, created_ms, attached_ms)\n\
         VALUES (?1, NULL, ?2, ?3, 0, 0, 0, ?4, ?5, 0)",
        params![id, original_key, thumbnail_key, timestamp, created_ms],
    )?;
    Ok(())
}

/// Completes a provisional row once both storage objects exist: the returned
/// keys and dimensions replace the provisional values and the row becomes a
/// normal claimable pending upload. A row the cleanup sweep fenced while the
/// PUTs were in flight cannot be completed any more: the caller gets the
/// standard "upload removed" error and must delete the objects it wrote.
pub fn complete_image_upload(
    connection: &Connection,
    id: &str,
    original_key: &str,
    thumbnail_key: &str,
    width: i64,
    height: i64,
) -> ApiResult<PostImageRow> {
    let updated = connection.execute(
        "UPDATE post_images SET original_key = ?1, thumbnail_key = ?2, width = ?3, height = ?4, attached_ms = NULL\n\
         WHERE id = ?5 AND post_id IS NULL AND attached_ms = 0",
        params![original_key, thumbnail_key, width, height, id],
    )?;
    if updated == 0 {
        return Err(claim_failure(connection, id)?);
    }
    let sql = format!("SELECT {IMAGE_COLUMNS} FROM post_images WHERE id = ?1");
    connection
        .query_row(&sql, params![id], image_from_row)
        .map_err(ApiError::from)
}

/// Turns a failed upload attempt into a durable cleanup record and returns
/// the row whose objects the caller may delete again. A still provisional row
/// is marked for immediate cleanup and a row that was swept while the request
/// was in flight is re-created from the deterministic keys; a row the sweep
/// already fenced is returned exactly as it is, so the caller retries the
/// deletion the fence never completed and the persisted keys stay intact.
/// Completed pending or attached rows are never touched (`None`).
pub fn mark_upload_failed(connection: &Connection, id: &str) -> ApiResult<Option<PostImageRow>> {
    let timestamp = iso_now();
    let created_ms = parse_js_date_ms(&timestamp).unwrap_or_else(|| now_unix() * 1_000);
    let (original_key, thumbnail_key) = crate::s3::image_keys(id)?;
    let changed = connection.execute(
        "INSERT INTO post_images (id, post_id, original_key, thumbnail_key, width, height, position, created_at, created_ms, attached_ms)\n\
         VALUES (?1, NULL, ?2, ?3, 0, 0, 0, ?4, ?5, ?5)\n\
         ON CONFLICT(id) DO UPDATE SET attached_ms = excluded.attached_ms\n\
         WHERE post_images.post_id IS NULL AND post_images.attached_ms = 0",
        params![id, original_key, thumbnail_key, timestamp, created_ms],
    )?;
    let sql = format!("SELECT {IMAGE_COLUMNS} FROM post_images WHERE id = ?1");
    let existing: Option<PostImageRow> = connection
        .query_row(&sql, params![id], image_from_row)
        .optional()?;
    // A fenced row is not un-fenced here: the sweep owns its objects, and a
    // positive marker set behind its back could be finished by a stale pass.
    Ok(match existing {
        Some(row)
            if row.post_id.is_none()
                && (changed > 0 || matches!(row.attached_ms, Some(ms) if ms < 0)) =>
        {
            Some(row)
        }
        _ => None,
    })
}

/// Marks an unattached, completed row for object cleanup and returns it, so
/// the caller can delete both storage objects first. Attached rows yield
/// `None` (the API refuses them), provisional in-flight rows and rows the
/// sweep already fenced are never touched, and rows already waiting for
/// cleanup are returned as-is.
pub fn take_image_for_cleanup(
    connection: &Connection,
    id: &str,
    now_ms: i64,
) -> ApiResult<Option<PostImageRow>> {
    let updated = connection.execute(
        "UPDATE post_images SET attached_ms = COALESCE(NULLIF(attached_ms, 0), ?1)\n\
         WHERE id = ?2 AND post_id IS NULL AND (attached_ms IS NULL OR attached_ms > 0)",
        params![now_ms, id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    let sql = format!("SELECT {IMAGE_COLUMNS} FROM post_images WHERE id = ?1");
    connection
        .query_row(&sql, params![id], image_from_row)
        .optional()
        .map_err(ApiError::from)
}

/// Rows whose objects still need deleting, oldest first, resuming after the
/// last candidate of a previous batch when `after` is given:
/// - rows already fenced by an earlier pass (`attached_ms < 0`) are retried;
/// - detached rows (`attached_ms > 0`, their post is gone or a cancel marked
///   them) are cleaned immediately;
/// - completed uploads never claimed past the pending TTL;
/// - provisional rows abandoned past the upload grace (partial or failed
///   attempts, including crashed processes).
///
/// The cursor makes one pass examine every eligible row even when a whole
/// batch can never be deleted: a failing row stays fenced for a later pass,
/// but it cannot starve the rows behind it.
pub fn list_image_cleanup_batch(
    connection: &Connection,
    now_ms: i64,
    after: Option<&ImageCleanupCursor>,
    limit: u32,
) -> ApiResult<Vec<PostImageRow>> {
    let grace_cutoff_ms = now_ms - IMAGE_UPLOAD_GRACE_MS;
    let cutoff_ms = now_ms - PENDING_IMAGE_TTL_MS;
    let sql = format!(
        "WITH candidates AS (\n\
             SELECT {IMAGE_COLUMNS},\n\
                    COALESCE(NULLIF(attached_ms, 0), created_ms) AS sort_key\n\
             FROM post_images\n\
             WHERE post_id IS NULL AND (\n\
                 attached_ms < 0\n\
                 OR attached_ms > 0\n\
                 OR (attached_ms = 0 AND created_ms < ?1)\n\
                 OR (attached_ms IS NULL AND created_ms < ?2)\n\
             )\n\
         )\n\
         SELECT {IMAGE_COLUMNS} FROM candidates\n\
         WHERE ?3 IS NULL OR sort_key > ?3 OR (sort_key = ?3 AND id > ?4)\n\
         ORDER BY sort_key ASC, id ASC\n\
         LIMIT ?5"
    );
    let (cursor_key, cursor_id) = match after {
        Some(cursor) => (Some(cursor.sort_key), Some(cursor.id.as_str())),
        None => (None, None),
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        params![grace_cutoff_ms, cutoff_ms, cursor_key, cursor_id, limit as i64],
        image_from_row,
    )?;
    let mut batch = Vec::new();
    for row in rows {
        batch.push(row?);
    }
    Ok(batch)
}

/// Atomically fences one listed cleanup candidate for the pass that is about
/// to delete its objects: the row moves to the cleanup-owned state
/// (`attached_ms < 0`) in the same statement that re-checks every eligibility
/// condition. An upload whose PUTs finished after the batch was listed can
/// therefore no longer complete the row (and a post can no longer claim it),
/// so objects are never deleted from under a live upload. A `None` answer
/// means the row stopped being a cleanup candidate and nothing may be deleted
/// for it.
///
/// Rows that are already fenced are fenced again and returned, which is how a
/// later pass retries a deletion that failed before. The persisted object
/// keys are never modified.
pub fn claim_image_cleanup(
    connection: &Connection,
    id: &str,
    now_ms: i64,
) -> ApiResult<Option<PostImageRow>> {
    let grace_cutoff_ms = now_ms - IMAGE_UPLOAD_GRACE_MS;
    let cutoff_ms = now_ms - PENDING_IMAGE_TTL_MS;
    let updated = connection.execute(
        "UPDATE post_images SET attached_ms = ?1\n\
         WHERE id = ?2 AND post_id IS NULL AND (\n\
             attached_ms < 0\n\
             OR attached_ms > 0\n\
             OR (attached_ms = 0 AND created_ms < ?3)\n\
             OR (attached_ms IS NULL AND created_ms < ?4)\n\
         )",
        params![IMAGE_CLEANUP_FENCED_MS, id, grace_cutoff_ms, cutoff_ms],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    let sql = format!("SELECT {IMAGE_COLUMNS} FROM post_images WHERE id = ?1");
    connection
        .query_row(&sql, params![id], image_from_row)
        .optional()
        .map_err(ApiError::from)
}

/// One `(sort_key, id)` position of a sweep pass: the cursor resumes listing
/// after the last candidate the pass examined, so rows that cannot be deleted
/// are retried later but never block the rest of the queue.
#[derive(Clone, Debug)]
pub struct ImageCleanupCursor {
    pub sort_key: i64,
    pub id: String,
}

impl ImageCleanupCursor {
    /// The cursor that resumes a pass after `image` exactly as it was listed;
    /// the key mirrors the `ORDER BY` expression of
    /// [`list_image_cleanup_batch`].
    pub fn after(image: &PostImageRow) -> Self {
        Self {
            sort_key: cleanup_sort_key(image),
            id: image.id.clone(),
        }
    }
}

/// Ordering key of one cleanup candidate: detached rows by their detach time,
/// fenced rows first (a negative marker orders before every timestamp) and
/// everything else by its upload creation time.
fn cleanup_sort_key(image: &PostImageRow) -> i64 {
    match image.attached_ms {
        Some(attached_ms) if attached_ms != 0 => attached_ms,
        _ => image.created_ms,
    }
}

/// Drops a row once both storage objects are confirmed gone. Guarded to rows
/// that already left the provisional state: cleanup must never delete an
/// attached image nor race a live upload whose storage PUTs are in flight.
pub fn finish_image_cleanup(connection: &Connection, id: &str) -> ApiResult<bool> {
    let removed = connection.execute(
        "DELETE FROM post_images WHERE id = ?1 AND post_id IS NULL AND (attached_ms IS NULL OR attached_ms != 0)",
        params![id],
    )?;
    Ok(removed > 0)
}


/// Stored optional rich document as SQLite keeps it: the empty string is the
/// "no description" sentinel, otherwise the row holds canonical document JSON.
fn description_value(stored: Option<&str>) -> ApiResult<Value> {
    let Some(raw) = stored.filter(|value| !value.is_empty()) else {
        return Ok(Value::Null);
    };
    serde_json::from_str(raw).map_err(|error| {
        ApiError::internal(format!("Stored province description is invalid JSON: {error}"))
    })
}

pub fn build_published_gubernia(
    connection: &Connection,
    canonical: &CanonicalProvinces,
    id: &str,
    slug: &str,
    description_json: Option<&str>,
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
    out.insert("description".to_string(), description_value(description_json)?);
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
    let row: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT id, slug, description_json FROM provinces WHERE published = 1 AND slug = ?1",
            params![slug],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match row {
        Some((id, slug, description_json)) => Ok(Some(build_published_gubernia(
            connection,
            canonical,
            &id,
            &slug,
            description_json.as_deref(),
        )?)),
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
    let sql = "SELECT s.id, s.province_id, s.name, s.uyezd_id, s.latitude, s.longitude, s.created_at, s.url, s.type, p.slug, p.description_json\n\
         FROM settlements s JOIN provinces p ON p.id = s.province_id\n\
         WHERE p.published = 1 AND COALESCE(s.url, '/naselennyy-punkt/' || s.id) = ?1\n\
         LIMIT 1";
    let row: Option<(SettlementRow, String, Option<String>)> = connection
        .query_row(&sql, params![url], |row| {
            Ok((settlement_from_row(row)?, row.get(9)?, row.get(10)?))
        })
        .optional()?;
    let Some((settlement, slug, description_json)) = row else {
        return Ok(None);
    };
    let gubernia = build_published_gubernia(
        connection,
        canonical,
        &settlement.province_id,
        &slug,
        description_json.as_deref(),
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
        "UPDATE provinces SET published = 1, slug = ?1, description_json = '' WHERE id = ?2",
        params![slug, id],
    )?;
    let gubernia = build_published_gubernia(&transaction, canonical, id, &slug, None)?;
    transaction.commit()?;
    Ok(gubernia)
}

pub fn update_gubernia_publication(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    id: &str,
    slug_value: &Value,
    description_value: Option<&Value>,
) -> ApiResult<Value> {
    let slug = validate_slug(slug_value)?;
    let description = validate_description(description_value)?;
    require_feature(canonical, id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, id)?;
    assert_unique_slug(&transaction, &slug, Some(id))?;
    let description_json = description
        .map(|document| serde_json::to_string(&document))
        .transpose()?;
    transaction.execute(
        "UPDATE provinces SET slug = ?1, description_json = ?2 WHERE id = ?3",
        params![slug, description_json.as_deref().unwrap_or(""), id],
    )?;
    let gubernia = build_published_gubernia(
        &transaction,
        canonical,
        id,
        &slug,
        description_json.as_deref(),
    )?;
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
        "UPDATE provinces SET published = 0, slug = NULL, description_json = '' WHERE id = ?1",
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
    let body = validate_optional_post_body(payload.get("body").unwrap_or(&Value::Null))?;
    let image_ids = validate_image_ids(payload)?;
    let body = resolve_post_body(body, !image_ids.is_empty())?;
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
    let mut post = PostRow {
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
        images: Vec::new(),
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
    // Claimed in the same transaction: a duplicate, unknown, expired or
    // already attached id aborts the whole creation (no partial post).
    let now_ms = now_unix() * 1_000;
    claim_post_images(&transaction, &post.id, &image_ids, now_ms)?;
    post.images = load_post_images(&transaction, &post.id)?;
    let value = post_value(&post)?;
    transaction.commit()?;
    Ok(value)
}

/// Saves an edited post so that its metadata, body, attachment set and
/// attachment order all live in one immediate transaction. `imageIds` is an
/// optional full ordered replacement ([`validate_image_ids_replacement`]): a
/// present list must name every image the post should keep, in order, plus
/// any fresh pending uploads to attach; every stored image left out is
/// detached with its durable cleanup marker, but only when the save commits.
///
/// Returns the serialized post and whether the save detached persisted
/// images, i.e. whether the caller has to run the storage cleanup sweep.
pub fn update_post(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    gubernia_id: &str,
    post_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<(Value, bool)> {
    let title = validate_post_title(payload.get("title").unwrap_or(&Value::Null))?;
    let body = validate_optional_post_body(payload.get("body").unwrap_or(&Value::Null))?;
    let replacement = validate_image_ids_replacement(payload)?;
    require_feature(canonical, gubernia_id)?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_published(&transaction, gubernia_id)?;
    let post = require_post(&transaction, gubernia_id, post_id)?;

    // The attachment replacement is resolved against the rows this immediate
    // transaction authoritatively owns, before the body rule and the metadata
    // write: an id that is unknown, expired, detached or attached to another
    // post fails the whole save here, and a replacement decides blank-body
    // validity exactly like creation does.
    let now_ms = now_unix() * 1_000;
    let (images, detached) = match &replacement {
        Some(image_ids) => {
            let detached =
                replace_post_images(&transaction, &post.id, image_ids, &post.images, now_ms)?;
            // Read the rows back from the transaction: the blank-body rule
            // and the response must reflect what the commit will persist.
            (load_post_images(&transaction, &post.id)?, detached)
        }
        // An absent field keeps the stored attachments untouched.
        None => (post.images.clone(), false),
    };
    let body = resolve_post_body(body, !images.is_empty())?;

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
        // The replacement's committed order, or the untouched attachments of
        // a request that omitted `imageIds` (they survive a cross-province
        // move because the post keeps its id).
        images,
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
        // The previous implementation deleted and re-inserted the row, which
        // detached post_images (ON DELETE SET NULL) and silently lost the
        // attachments; a single UPDATE keeps the post id and its images.
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
            "UPDATE posts SET province_id = ?1, title = ?2, body_json = ?3, uyezd_id = ?4,\n\
             settlement_id = ?5, year = ?6, archive_reference = ?7, category = ?8,\n\
             updated_at = ?9, position = ?10\n\
             WHERE id = ?11 AND province_id = ?12",
            params![
                updated.province_id,
                updated.title,
                updated.body_json,
                updated.uyezd_id,
                updated.settlement_id,
                updated.year,
                updated.archive_reference,
                updated.category,
                updated.updated_at,
                position,
                updated.id,
                gubernia_id,
            ],
        )?;
    }
    let value = post_value(&updated)?;
    transaction.commit()?;
    // `detached` reports stored images this save moved to the cleanup state;
    // their storage objects may only be deleted after this commit.
    Ok((value, detached))
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

    // -----------------------------------------------------------------------
    // Post images: validation, claiming and cleanup bookkeeping
    // -----------------------------------------------------------------------

    /// Minimal schema for the image lifecycle: the posts table (foreign keys
    /// stay off on this in-memory connection) plus the migration-4 table.
    fn image_test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("../migrations/0001_init.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../migrations/0004_post_images.sql"))
            .unwrap();
        connection
    }

    fn insert_test_pending(connection: &Connection, id: &str, created_ms: i64) {
        connection
            .execute(
                "INSERT INTO post_images (id, post_id, original_key, thumbnail_key, width, height, position, created_at, created_ms, attached_ms)\n\
                 VALUES (?1, NULL, ?2, ?3, 10, 10, 0, '2026-10-09T00:00:00.000Z', ?4, NULL)",
                rusqlite::params![
                    id,
                    format!("post-images/{id}/original"),
                    format!("post-images/{id}/thumbnail"),
                    created_ms,
                ],
            )
            .unwrap();
    }

    /// Minimal province + post so claiming can satisfy the real foreign key.
    fn insert_test_post(connection: &Connection, post_id: &str) {
        // The version-1 provinces column still holds the legacy plain-text
        // description name; migration 3 renames it in real databases.
        connection
            .execute(
                "INSERT OR IGNORE INTO provinces (id, published, slug, description)\n\
                 VALUES ('province-1', 1, 'province-1', '')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO posts (id, province_id, title, body_json, created_at, updated_at, created_ms, position)\n\
                 VALUES (?1, 'province-1', 'Тест', '{}', '2026-10-09T00:00:00.000Z', '2026-10-09T00:00:00.000Z', 0, 0)",
                rusqlite::params![post_id],
            )
            .unwrap();
    }

    #[test]
    fn image_id_lists_are_bounded_and_unique() {
        assert_eq!(
            validate_image_ids(&Map::new()).unwrap(),
            Vec::<String>::new()
        );
        let ordered = json!({ "imageIds": ["b", "a"] });
        assert_eq!(
            validate_image_ids(ordered.as_object().unwrap()).unwrap(),
            vec!["b".to_string(), "a".to_string()]
        );
        let duplicated = json!({ "imageIds": ["a", "a"] });
        assert_eq!(
            validate_image_ids(duplicated.as_object().unwrap())
                .unwrap_err()
                .message,
            "Идентификаторы изображений не должны повторяться."
        );
        let not_array = json!({ "imageIds": "a" });
        assert_eq!(
            validate_image_ids(not_array.as_object().unwrap())
                .unwrap_err()
                .message,
            "Список изображений должен быть массивом."
        );
        let not_string = json!({ "imageIds": [1] });
        assert_eq!(
            validate_image_ids(not_string.as_object().unwrap())
                .unwrap_err()
                .message,
            "Некорректный идентификатор изображения."
        );
        let eleven: Vec<String> = (0..11).map(|index| format!("img-{index}")).collect();
        let overflow = json!({ "imageIds": eleven });
        assert_eq!(
            validate_image_ids(overflow.as_object().unwrap())
                .unwrap_err()
                .message,
            "К публикации можно приложить не более 10 изображений."
        );
    }

    #[test]
    fn patch_image_ids_replacement_distinguishes_omission_null_and_empty() {
        assert_eq!(validate_image_ids_replacement(&Map::new()).unwrap(), None);
        let null = json!({ "imageIds": null });
        assert_eq!(
            validate_image_ids_replacement(null.as_object().unwrap())
                .unwrap_err()
                .message,
            "Список изображений должен быть массивом."
        );
        let empty = json!({ "imageIds": [] });
        assert_eq!(
            validate_image_ids_replacement(empty.as_object().unwrap()).unwrap(),
            Some(Vec::new())
        );
        let ordered = json!({ "imageIds": ["b", "a"] });
        assert_eq!(
            validate_image_ids_replacement(ordered.as_object().unwrap()).unwrap(),
            Some(vec!["b".to_string(), "a".to_string()])
        );
    }

    #[test]
    fn replacement_retains_claims_orders_and_detaches_atomically() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");
        insert_test_pending(&connection, "keep", now_ms);
        insert_test_pending(&connection, "drop", now_ms);
        insert_test_pending(&connection, "fresh", now_ms);

        // The baseline attachments are committed first, so the replacement's
        // rollback is observed against a stable stored state.
        let baseline = connection.transaction().unwrap();
        claim_post_images(
            &baseline,
            "post-1",
            &["keep".to_string(), "drop".to_string()],
            now_ms,
        )
        .unwrap();
        baseline.commit().unwrap();

        let transaction = connection.transaction().unwrap();
        let current = load_post_images(&transaction, "post-1").unwrap();
        let detached = replace_post_images(
            &transaction,
            "post-1",
            &["drop".to_string(), "fresh".to_string()],
            &current,
            now_ms,
        )
        .unwrap();
        assert!(detached);
        let images = load_post_images(&transaction, "post-1").unwrap();
        assert_eq!(
            images.iter().map(|image| image.id.as_str()).collect::<Vec<_>>(),
            vec!["drop", "fresh"]
        );
        assert_eq!((images[0].position, images[1].position), (0, 1));
        assert!(!replace_post_images(
            &transaction,
            "post-1",
            &["fresh".to_string(), "drop".to_string()],
            &images,
            now_ms,
        )
        .unwrap());
        let kept = load_image_access(&transaction, "keep").unwrap().unwrap().image;
        assert_eq!(kept.post_id, None);
        assert!(kept.attached_ms.is_some_and(|ms| ms > 0));

        // Nothing of the replacement may survive a rollback: the committed
        // attachments and their order are exactly as before the save.
        transaction.rollback().unwrap();
        let images = load_post_images(&connection, "post-1").unwrap();
        assert_eq!(
            images.iter().map(|image| image.id.as_str()).collect::<Vec<_>>(),
            vec!["keep", "drop"]
        );
        let kept = load_image_access(&connection, "keep").unwrap().unwrap().image;
        assert_eq!(kept.post_id.as_deref(), Some("post-1"));
        let fresh = load_image_access(&connection, "fresh").unwrap().unwrap().image;
        assert_eq!(fresh.post_id, None);
        assert_eq!(fresh.attached_ms, None);
    }

    #[test]
    fn replacement_failure_rolls_back_earlier_claims() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");
        insert_test_pending(&connection, "owner", now_ms);
        insert_test_pending(&connection, "fresh", now_ms);

        // The stored attachment is committed first: only then does the
        // failed replacement's rollback have a stable baseline.
        let baseline = connection.transaction().unwrap();
        claim_post_images(&baseline, "post-1", &["owner".to_string()], now_ms).unwrap();
        baseline.commit().unwrap();

        let transaction = connection.transaction().unwrap();
        let current = load_post_images(&transaction, "post-1").unwrap();
        let failure = replace_post_images(
            &transaction,
            "post-1",
            &["fresh".to_string(), "missing".to_string()],
            &current,
            now_ms,
        )
        .unwrap_err();
        assert_eq!(failure.message, "Изображение не найдено.");
        // The claim that ran before the failing id did happen inside the
        // transaction, and the error left the stored attachment attached...
        let claimed = load_image_access(&transaction, "fresh").unwrap().unwrap().image;
        assert_eq!(claimed.post_id.as_deref(), Some("post-1"));
        let owner = load_image_access(&transaction, "owner").unwrap().unwrap().image;
        assert_eq!(owner.post_id.as_deref(), Some("post-1"));
        transaction.rollback().unwrap();

        // ...and it is gone again once the request is abandoned, while the
        // stored attachment was never detached.
        let fresh = load_image_access(&connection, "fresh").unwrap().unwrap().image;
        assert_eq!(fresh.post_id, None);
        assert_eq!(fresh.attached_ms, None);
        let images = load_post_images(&connection, "post-1").unwrap();
        assert_eq!(
            images.iter().map(|image| image.id.as_str()).collect::<Vec<_>>(),
            vec!["owner"]
        );
    }

    #[test]
    fn claim_attaches_pending_images_in_submitted_order() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");
        insert_test_pending(&connection, "second", now_ms);
        insert_test_pending(&connection, "first", now_ms);

        let transaction = connection.transaction().unwrap();
        claim_post_images(
            &transaction,
            "post-1",
            &["second".to_string(), "first".to_string()],
            now_ms,
        )
        .unwrap();
        transaction.commit().unwrap();

        let mut statement = connection
            .prepare("SELECT id, post_id, position, attached_ms FROM post_images ORDER BY position ASC")
            .unwrap();
        let rows: Vec<(String, Option<String>, i64, Option<i64>)> = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "second".to_string(),
                    Some("post-1".to_string()),
                    0,
                    Some(now_ms)
                ),
                (
                    "first".to_string(),
                    Some("post-1".to_string()),
                    1,
                    Some(now_ms)
                ),
            ]
        );
    }

    #[test]
    fn claim_rejects_attached_expired_and_unknown_ids() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");
        insert_test_post(&connection, "post-2");
        insert_test_pending(&connection, "fresh", now_ms);
        insert_test_pending(
            &connection,
            "expired",
            now_ms - PENDING_IMAGE_TTL_MS - 1,
        );
        insert_test_pending(&connection, "detached", now_ms);
        connection
            .execute(
                "UPDATE post_images SET attached_ms = ?1 WHERE id = 'detached'",
                rusqlite::params![now_ms],
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        claim_post_images(&transaction, "post-1", &["fresh".to_string()], now_ms).unwrap();
        assert_eq!(
            claim_failure(&transaction, "fresh").unwrap().message,
            "Изображение уже привязано к публикации."
        );
        assert_eq!(
            claim_failure(&transaction, "expired").unwrap().message,
            "Срок загрузки изображения истёк."
        );
        assert_eq!(
            claim_failure(&transaction, "detached").unwrap().message,
            "Изображение не найдено: загрузка удалена."
        );
        assert_eq!(
            claim_failure(&transaction, "missing").unwrap().message,
            "Изображение не найдено."
        );
        // The failed claim for "expired" never touched its row.
        assert_eq!(
            claim_post_images(&transaction, "post-2", &["expired".to_string()], now_ms)
                .unwrap_err()
                .message,
            "Срок загрузки изображения истёк."
        );
    }

    #[test]
    fn upload_lifecycle_writes_a_durable_provisional_row() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");

        // Before the storage PUTs the row only carries the deterministic keys
        // and the in-flight marker, so partial uploads are always tracked.
        begin_image_upload(&connection, "img-1").unwrap();
        let row = load_image_access(&connection, "img-1").unwrap().unwrap().image;
        assert_eq!(row.attached_ms, Some(0));
        assert_eq!(row.width, 0);
        assert_eq!(row.height, 0);
        assert_eq!(row.original_key, "post-images/img-1/original");
        assert_eq!(row.thumbnail_key, "post-images/img-1/thumbnail");
        // In-flight and abandoned rows are never claimable nor user-deletable.
        assert!(
            list_image_cleanup_batch(&connection, now_ms, None, 10)
                .unwrap()
                .is_empty()
        );
        assert!(take_image_for_cleanup(&connection, "img-1", now_ms)
            .unwrap()
            .is_none());

        // Completion turns it into a normal claimable upload with the keys and
        // dimensions returned by the storage layer.
        let completed = complete_image_upload(
            &connection,
            "img-1",
            "post-images/img-1/original",
            "post-images/img-1/thumbnail",
            800,
            600,
        )
        .unwrap();
        assert_eq!(completed.attached_ms, None);
        assert_eq!(completed.width, 800);
        assert_eq!(completed.height, 600);
        let transaction = connection.transaction().unwrap();
        claim_post_images(&transaction, "post-1", &["img-1".to_string()], now_ms).unwrap();
        transaction.commit().unwrap();
        assert_eq!(
            load_image_access(&connection, "img-1")
                .unwrap()
                .unwrap()
                .image
                .post_id
                .as_deref(),
            Some("post-1")
        );
    }

    #[test]
    fn failed_uploads_keep_their_cleanup_keys() {
        let connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");

        // A provisional row whose PUT failed becomes an immediate cleanup
        // candidate instead of losing the partial object.
        begin_image_upload(&connection, "img-fail").unwrap();
        let marked = mark_upload_failed(&connection, "img-fail").unwrap().unwrap();
        // `begin_image_upload` and `mark_upload_failed` read the wall clock
        // independently, so the marker is its own timestamp, not provably
        // equal to `created_ms`. What matters is that it is a positive marker
        // and that the sweep below acts on it.
        assert!(
            marked.attached_ms.is_some_and(|ms| ms > 0),
            "a failed provisional upload is marked for immediate cleanup"
        );
        assert_eq!(marked.original_key, "post-images/img-fail/original");
        let batch = list_image_cleanup_batch(&connection, now_ms, None, 10).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].id, "img-fail");

        // A row swept while the request was in flight is re-created from the
        // deterministic keys, so the retry can still find the objects.
        let recreated = mark_upload_failed(&connection, "img-swept").unwrap().unwrap();
        assert_eq!(recreated.original_key, "post-images/img-swept/original");
        assert_eq!(recreated.thumbnail_key, "post-images/img-swept/thumbnail");
        // Re-creation writes both timestamps from one bound parameter, so
        // this equality stays deterministic.
        assert_eq!(recreated.attached_ms, Some(recreated.created_ms));

        // A completed pending upload is claimable: a late failure report must
        // not mark it for deletion.
        insert_test_pending(&connection, "img-ready", now_ms);
        assert!(mark_upload_failed(&connection, "img-ready").unwrap().is_none());
        let ready = load_image_access(&connection, "img-ready").unwrap().unwrap().image;
        assert_eq!(ready.attached_ms, None);
        assert_eq!(ready.width, 10);
    }

    #[test]
    fn cleanup_batch_covers_orphans_and_expired_pending_rows() {
        let connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");
        insert_test_pending(&connection, "fresh", now_ms);
        insert_test_pending(&connection, "expired", now_ms - PENDING_IMAGE_TTL_MS - 1);
        insert_test_pending(&connection, "detached", now_ms - 500);
        connection
            .execute(
                "UPDATE post_images SET attached_ms = ?1 WHERE id = 'detached'",
                rusqlite::params![now_ms - 100],
            )
            .unwrap();
        // An abandoned provisional row is only swept after the upload grace.
        begin_image_upload(&connection, "in-flight").unwrap();
        connection
            .execute(
                "UPDATE post_images SET created_ms = ?1 WHERE id = 'in-flight'",
                rusqlite::params![now_ms],
            )
            .unwrap();
        begin_image_upload(&connection, "abandoned").unwrap();
        connection
            .execute(
                "UPDATE post_images SET created_ms = ?1 WHERE id = 'abandoned'",
                rusqlite::params![now_ms - IMAGE_UPLOAD_GRACE_MS - 1],
            )
            .unwrap();

        let batch = list_image_cleanup_batch(&connection, now_ms, None, 10).unwrap();
        let ids: Vec<&str> = batch.iter().map(|image| image.id.as_str()).collect();
        // Oldest first: expired, then the abandoned upload, then the detached row.
        assert_eq!(ids, vec!["expired", "abandoned", "detached"]);
        assert_eq!(batch[0].original_key, "post-images/expired/original");

        // Fresh pending uploads stay claimable; dropping rows is guarded.
        assert!(finish_image_cleanup(&connection, "fresh").unwrap());
        assert!(!finish_image_cleanup(&connection, "fresh").unwrap());
        connection
            .execute(
                "UPDATE post_images SET post_id = 'post-1' WHERE id = 'expired'",
                [],
            )
            .unwrap();
        assert!(
            !finish_image_cleanup(&connection, "expired").unwrap(),
            "attached rows must survive cleanup"
        );
    }

    // -----------------------------------------------------------------------
    // Post images: cleanup fencing, rollback bookkeeping and queue progress
    // -----------------------------------------------------------------------

    /// The sweep fences a row before touching S3: depending on who wins the
    /// race, either the upload completes and the row is left alone, or the
    /// fence wins and the upload can no longer complete (nor be claimed).
    #[test]
    fn cleanup_fence_wins_or_loses_the_completion_race_atomically() {
        let mut connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");

        // The PUTs finished after the row was listed: completion won, so the
        // freshly pending upload must not be fenced nor swept.
        begin_image_upload(&connection, "img-finished").unwrap();
        connection
            .execute(
                "UPDATE post_images SET created_ms = ?1 WHERE id = 'img-finished'",
                rusqlite::params![now_ms - IMAGE_UPLOAD_GRACE_MS - 1],
            )
            .unwrap();
        let completed = complete_image_upload(
            &connection,
            "img-finished",
            "post-images/img-finished/original",
            "post-images/img-finished/thumbnail",
            800,
            600,
        )
        .unwrap();
        assert_eq!(completed.attached_ms, None);
        assert!(claim_image_cleanup(&connection, "img-finished", now_ms)
            .unwrap()
            .is_none());

        // The sweep won instead: the provisional row is fenced with its
        // deterministic keys, so the upload can no longer complete it.
        begin_image_upload(&connection, "img-fenced").unwrap();
        connection
            .execute(
                "UPDATE post_images SET created_ms = ?1 WHERE id = 'img-fenced'",
                rusqlite::params![now_ms - IMAGE_UPLOAD_GRACE_MS - 1],
            )
            .unwrap();
        let fenced = claim_image_cleanup(&connection, "img-fenced", now_ms)
            .unwrap()
            .expect("the abandoned provisional row is fenced");
        assert_eq!(fenced.attached_ms, Some(IMAGE_CLEANUP_FENCED_MS));
        assert_eq!(fenced.original_key, "post-images/img-fenced/original");
        assert_eq!(fenced.thumbnail_key, "post-images/img-fenced/thumbnail");

        // Completion loses the compare-and-set: the error says the upload was
        // removed, and the fence with the persisted keys is untouched.
        let lost = complete_image_upload(
            &connection,
            "img-fenced",
            "post-images/img-fenced/original",
            "post-images/img-fenced/thumbnail",
            800,
            600,
        )
        .err()
        .expect("a fenced row cannot be completed");
        assert_eq!(lost.message, "Изображение не найдено: загрузка удалена.");
        let stored = load_image_access(&connection, "img-fenced")
            .unwrap()
            .unwrap()
            .image;
        assert_eq!(stored.attached_ms, Some(IMAGE_CLEANUP_FENCED_MS));
        assert_eq!(stored.original_key, "post-images/img-fenced/original");
        assert_eq!(stored.width, 0, "a fenced row never becomes a pending upload");

        // A fenced row can be claimed by no post and cancelled by no user.
        let transaction = connection.transaction().unwrap();
        assert_eq!(
            claim_post_images(&transaction, "post-1", &["img-fenced".to_string()], now_ms)
                .unwrap_err()
                .message,
            "Изображение не найдено: загрузка удалена."
        );
        transaction.rollback().unwrap();
        assert!(take_image_for_cleanup(&connection, "img-fenced", now_ms)
            .unwrap()
            .is_none());

        // A later pass re-fences the same row for the retry, and only the
        // confirmed object deletion drops it.
        let retried = claim_image_cleanup(&connection, "img-fenced", now_ms + 1)
            .unwrap()
            .expect("fenced rows are re-claimed for retry");
        assert_eq!(retried.attached_ms, Some(IMAGE_CLEANUP_FENCED_MS));
        assert_eq!(retried.original_key, "post-images/img-fenced/original");
        assert!(finish_image_cleanup(&connection, "img-fenced").unwrap());
        assert!(load_image_access(&connection, "img-fenced").unwrap().is_none());
    }

    /// A failed upload keeps its deterministic keys until the deletion works;
    /// after a successful rollback both objects are gone, so the row is
    /// dropped right away instead of lingering until a sweep.
    #[test]
    fn failed_upload_rows_are_dropped_only_after_a_successful_rollback() {
        let connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        insert_test_post(&connection, "post-1");

        // A failed PUT: the provisional row becomes an immediate cleanup
        // record, and a successful deletion lets the caller drop it.
        begin_image_upload(&connection, "img-rollback").unwrap();
        let marked = mark_upload_failed(&connection, "img-rollback")
            .unwrap()
            .expect("the provisional row is marked for cleanup");
        assert!(matches!(marked.attached_ms, Some(ms) if ms > 0));
        assert_eq!(marked.original_key, "post-images/img-rollback/original");
        assert_eq!(marked.thumbnail_key, "post-images/img-rollback/thumbnail");
        assert!(finish_image_cleanup(&connection, "img-rollback").unwrap());
        assert!(load_image_access(&connection, "img-rollback").unwrap().is_none());

        // A failed deletion: the same transition, but the row is kept with its
        // keys and the very next sweep can retry it immediately.
        begin_image_upload(&connection, "img-retry").unwrap();
        let marked = mark_upload_failed(&connection, "img-retry").unwrap().unwrap();
        assert_eq!(marked.original_key, "post-images/img-retry/original");
        let batch = list_image_cleanup_batch(&connection, now_ms, None, 10).unwrap();
        assert_eq!(
            batch.iter().map(|image| image.id.as_str()).collect::<Vec<_>>(),
            vec!["img-retry"]
        );
        let fenced = claim_image_cleanup(&connection, "img-retry", now_ms)
            .unwrap()
            .expect("the durable record is swept");
        assert_eq!(fenced.original_key, "post-images/img-retry/original");
        assert!(finish_image_cleanup(&connection, "img-retry").unwrap());

        // A row the sweep fenced while the request was in flight is handed
        // back untouched, so the caller retries the deletion the fence never
        // completed and the persisted keys stay exactly as they are.
        begin_image_upload(&connection, "img-race").unwrap();
        connection
            .execute(
                "UPDATE post_images SET created_ms = ?1 WHERE id = 'img-race'",
                rusqlite::params![now_ms - IMAGE_UPLOAD_GRACE_MS - 1],
            )
            .unwrap();
        assert!(claim_image_cleanup(&connection, "img-race", now_ms)
            .unwrap()
            .is_some());
        let fenced = mark_upload_failed(&connection, "img-race")
            .unwrap()
            .expect("a fenced row is handed back for the retry");
        assert_eq!(fenced.attached_ms, Some(IMAGE_CLEANUP_FENCED_MS));
        assert_eq!(fenced.original_key, "post-images/img-race/original");
        assert_eq!(fenced.thumbnail_key, "post-images/img-race/thumbnail");
        assert!(finish_image_cleanup(&connection, "img-race").unwrap());

        // A row that vanished is re-created from the deterministic keys.
        let recreated = mark_upload_failed(&connection, "img-swept").unwrap().unwrap();
        assert_eq!(recreated.original_key, "post-images/img-swept/original");
        assert_eq!(recreated.thumbnail_key, "post-images/img-swept/thumbnail");
        assert!(recreated.attached_ms.is_some());

        // Live rows are never touched: a completed pending upload and an
        // attached image; neither is swept nor dropped.
        insert_test_pending(&connection, "img-ready", now_ms);
        assert!(mark_upload_failed(&connection, "img-ready").unwrap().is_none());
        insert_test_pending(&connection, "img-attached", now_ms);
        connection
            .execute(
                "UPDATE post_images SET post_id = 'post-1', attached_ms = ?1 WHERE id = 'img-attached'",
                rusqlite::params![now_ms],
            )
            .unwrap();
        assert!(mark_upload_failed(&connection, "img-attached")
            .unwrap()
            .is_none());
        assert!(claim_image_cleanup(&connection, "img-attached", now_ms)
            .unwrap()
            .is_none());
        assert!(!finish_image_cleanup(&connection, "img-attached").unwrap());

        // An in-flight provisional row is not swept either.
        begin_image_upload(&connection, "img-live").unwrap();
        assert!(claim_image_cleanup(&connection, "img-live", now_ms)
            .unwrap()
            .is_none());
        assert!(!finish_image_cleanup(&connection, "img-live").unwrap());
    }

    /// One pass must walk past a batch of rows whose deletion always fails:
    /// otherwise a handful of poison ids starve every row behind them.
    #[test]
    fn cleanup_pass_advances_past_persistently_failing_rows() {
        let connection = image_test_connection();
        let now_ms = 1_000_000_000_000i64;
        const BATCH: u32 = 8;
        insert_test_post(&connection, "post-1");
        let mut poison: Vec<String> = Vec::new();
        for index in 0..32 {
            let id = format!("poison-{index:02}");
            insert_test_pending(&connection, &id, now_ms - 1_000);
            connection
                .execute(
                    "UPDATE post_images SET attached_ms = ?1 WHERE id = ?2",
                    rusqlite::params![now_ms - 1_000, id],
                )
                .unwrap();
            poison.push(id);
        }
        // A newer detached row whose deletion works sorts behind every poison
        // row, so only a pass that advances past a failing batch reaches it.
        insert_test_pending(&connection, "healthy", now_ms - 1_000);
        connection
            .execute(
                "UPDATE post_images SET attached_ms = ?1 WHERE id = 'healthy'",
                rusqlite::params![now_ms - 900],
            )
            .unwrap();

        // The pass mirrors `sweep_image_objects`: list behind the cursor,
        // fence each candidate, "fail" the deletion of the poison ids and
        // drop everything else.
        let mut cursor: Option<ImageCleanupCursor> = None;
        let mut attempted: Vec<String> = Vec::new();
        let mut finished: Vec<String> = Vec::new();
        loop {
            let batch =
                list_image_cleanup_batch(&connection, now_ms, cursor.as_ref(), BATCH).unwrap();
            let Some(last) = batch.last() else {
                break;
            };
            cursor = Some(ImageCleanupCursor::after(last));
            for image in batch {
                let claimed = claim_image_cleanup(&connection, &image.id, now_ms)
                    .unwrap()
                    .expect("a listed candidate is claimable");
                attempted.push(claimed.id.clone());
                if poison.contains(&claimed.id) {
                    // The storage deletion failed; the row stays fenced.
                    continue;
                }
                if finish_image_cleanup(&connection, &claimed.id).unwrap() {
                    finished.push(claimed.id);
                }
            }
        }

        assert_eq!(finished, vec!["healthy".to_string()]);
        assert_eq!(
            attempted.len(),
            poison.len() + 1,
            "every candidate is attempted once per pass"
        );
        assert_eq!(attempted.last().map(String::as_str), Some("healthy"));
        // The poison rows are still there, still fenced, keys intact.
        for id in &poison {
            let row = load_image_access(&connection, id).unwrap().unwrap().image;
            assert_eq!(row.attached_ms, Some(IMAGE_CLEANUP_FENCED_MS));
            assert_eq!(row.original_key, format!("post-images/{id}/original"));
            assert_eq!(row.thumbnail_key, format!("post-images/{id}/thumbnail"));
        }
        // A later pass retries the fenced rows first and terminates.
        let retry =
            list_image_cleanup_batch(&connection, now_ms, None, poison.len() as u32).unwrap();
        assert_eq!(retry.len(), poison.len());
        assert!(poison.contains(&retry[0].id));
    }
}
