use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Map, Value};

use crate::content::MAX_POST_DOCUMENT_JSON_CHARACTERS;
use crate::error::{ApiError, ApiResult};
use crate::http;
use crate::session;
use crate::store;
use crate::util::{now_unix, utf16_len};
use crate::AppState;

/// `MAX_POST_REQUEST_BYTES = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 2_000`,
/// plus the 8 KiB placement-metadata margin from the post routes.
const MAX_POST_REQUEST_BYTES: usize = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 2_000;
const POST_REQUEST_BYTES: usize = MAX_POST_REQUEST_BYTES + 8_192;
/// Province PATCH carries a rich description document plus the slug, so it
/// keeps the same document bound as the post routes.
const GUBERNIA_REQUEST_BYTES: usize = MAX_POST_REQUEST_BYTES + 1_024;
const SETTLEMENT_REQUEST_BYTES: usize = 4_096;
const SETTINGS_REQUEST_BYTES: usize = 2_048;

fn exact_keys(payload: &Map<String, Value>, expected: &[&str]) -> bool {
    payload.len() == expected.len() && payload.keys().all(|key| expected.contains(&key.as_str()))
}

fn read_kind(value: Option<&Value>) -> ApiResult<String> {
    match value.and_then(Value::as_str) {
        Some(kind) if kind == "categories" || kind == "settlementTypes" => Ok(kind.to_string()),
        _ => Err(ApiError::bad_request("Неизвестный список настроек.")),
    }
}

fn attach_cookie(response: &mut Response, value: String) -> ApiResult<()> {
    let header_value = HeaderValue::from_str(&value).map_err(ApiError::internal)?;
    response.headers_mut().insert(header::SET_COOKIE, header_value);
    Ok(())
}

// ---------------------------------------------------------------------------
// Browser API: gubernias
// ---------------------------------------------------------------------------

pub async fn gubernias_get(State(state): State<AppState>) -> ApiResult<Response> {
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::get_gubernias_collection(conn, &canonical))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}

pub async fn gubernias_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, None).await?;
    let id = match payload.get("id") {
        Some(Value::String(id))
            if !id.is_empty() && utf16_len(id) <= store::MAX_ID_LENGTH =>
        {
            id.clone()
        }
        _ => return Err(ApiError::bad_request("Некорректный идентификатор губернии.")),
    };
    let slug = payload.get("slug").cloned().unwrap_or(Value::Null);
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::publish_gubernia(conn, &canonical, &id, &slug))
        .await?;
    Ok(http::json_response(StatusCode::CREATED, value))
}

pub async fn gubernia_patch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(GUBERNIA_REQUEST_BYTES)).await?;
    let slug = payload.get("slug").cloned().unwrap_or(Value::Null);
    let description = payload.get("description").cloned();
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| {
            store::update_gubernia_publication(
                conn,
                &canonical,
                &id,
                &slug,
                description.as_ref(),
            )
        })
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}

pub async fn gubernia_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let canonical = state.canonical.clone();
    state
        .call(move |conn| store::unpublish_gubernia(conn, &canonical, &id))
        .await?;
    Ok(http::empty_response(StatusCode::NO_CONTENT))
}

// ---------------------------------------------------------------------------
// Browser API: posts
// ---------------------------------------------------------------------------

pub async fn post_create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(POST_REQUEST_BYTES)).await?;
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::create_post(conn, &canonical, &id, &payload))
        .await?;
    Ok(http::json_response(StatusCode::CREATED, value))
}

pub async fn post_update(
    State(state): State<AppState>,
    Path((id, post_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(POST_REQUEST_BYTES)).await?;
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::update_post(conn, &canonical, &id, &post_id, &payload))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}

pub async fn post_delete(
    State(state): State<AppState>,
    Path((id, post_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let canonical = state.canonical.clone();
    state
        .call(move |conn| store::delete_post(conn, &canonical, &id, &post_id))
        .await?;
    Ok(http::empty_response(StatusCode::NO_CONTENT))
}

// ---------------------------------------------------------------------------
// Browser API: settlements
// ---------------------------------------------------------------------------

pub async fn settlement_create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(SETTLEMENT_REQUEST_BYTES)).await?;
    let canonical = state.canonical.clone();
    let geo = state.geo.clone();
    let value = state
        .call(move |conn| store::create_settlement(conn, &canonical, &geo, &id, &payload))
        .await?;
    Ok(http::json_response(StatusCode::CREATED, value))
}

pub async fn settlement_delete(
    State(state): State<AppState>,
    Path((id, settlement_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let canonical = state.canonical.clone();
    let target = settlement_id.clone();
    state
        .call(move |conn| store::delete_settlement(conn, &canonical, &id, &target))
        .await?;
    // The publication is already gone by the time the reference is cleaned up,
    // so a stale reference block must not turn a successful removal into a
    // failure the administration would retry: log it and report success.
    if let Err(error) = state
        .call(move |conn| store::remove_settlement_reference(conn, &settlement_id))
        .await
    {
        eprintln!("Settlement reference cleanup failed: {}", error.message);
    }
    Ok(http::empty_response(StatusCode::NO_CONTENT))
}

// ---------------------------------------------------------------------------
// Browser API: settlement reference and about content
// ---------------------------------------------------------------------------

pub async fn reference_patch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(MAX_POST_REQUEST_BYTES)).await?;
    if !exact_keys(&payload, &["body"]) {
        return Err(ApiError::bad_request("Ожидается документ содержимого страницы."));
    }
    let document = payload.get("body").cloned().unwrap_or(Value::Null);
    let value = state
        .call(move |conn| {
            let Some(settlement_id) =
                store::find_published_settlement_id_by_slug(conn, &slug)?
            else {
                return Err(ApiError::not_found("Населённый пункт не найден."));
            };
            store::save_settlement_reference(conn, &settlement_id, &document)
        })
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "body": value })))
}

pub async fn about_patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(MAX_POST_REQUEST_BYTES)).await?;
    if !exact_keys(&payload, &["body"]) {
        return Err(ApiError::bad_request("Ожидается документ содержимого страницы."));
    }
    let document = payload.get("body").cloned().unwrap_or(Value::Null);
    let value = state
        .call(move |conn| store::save_about_content(conn, &document))
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "body": value })))
}

// ---------------------------------------------------------------------------
// Browser API: settings
// ---------------------------------------------------------------------------

pub async fn settings_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(SETTINGS_REQUEST_BYTES)).await?;
    if !exact_keys(&payload, &["kind", "name"]) {
        return Err(ApiError::bad_request("Ожидается название и список настроек."));
    }
    let kind = read_kind(payload.get("kind"))?;
    let name = payload.get("name").cloned().unwrap_or(Value::Null);
    let value = state
        .call(move |conn| store::add_site_setting(conn, &kind, &name))
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "settings": value })))
}

pub async fn settings_patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(SETTINGS_REQUEST_BYTES)).await?;
    if !exact_keys(&payload, &["kind", "originalName", "name"]) {
        return Err(ApiError::bad_request(
            "Ожидается исходное название, новое название и список настроек.",
        ));
    }
    let kind = read_kind(payload.get("kind"))?;
    let original_name = payload.get("originalName").cloned().unwrap_or(Value::Null);
    let name = payload.get("name").cloned().unwrap_or(Value::Null);
    let value = state
        .call(move |conn| store::rename_site_setting(conn, &kind, &original_name, &name))
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "settings": value })))
}

pub async fn settings_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let payload = http::read_json_object(&headers, body, Some(SETTINGS_REQUEST_BYTES)).await?;
    if !exact_keys(&payload, &["kind", "name"]) {
        return Err(ApiError::bad_request("Ожидается название и список настроек."));
    }
    let kind = read_kind(payload.get("kind"))?;
    let name = payload.get("name").cloned().unwrap_or(Value::Null);
    let value = state
        .call(move |conn| store::delete_site_setting(conn, &kind, &name))
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "settings": value })))
}

// ---------------------------------------------------------------------------
// Browser API: admin session
// ---------------------------------------------------------------------------

pub async fn session_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    let value = http::read_json_any(&headers, body).await?;
    let (login, password) = match &value {
        Value::Object(map) => {
            let login = match map.get("login") {
                Some(Value::String(login)) if utf16_len(login) <= 128 => login.clone(),
                _ => return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Неверный логин или пароль.")),
            };
            let password = match map.get("password") {
                Some(Value::String(password)) if utf16_len(password) <= 128 => password.clone(),
                _ => return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Неверный логин или пароль.")),
            };
            (login, password)
        }
        Value::Array(_) => {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Неверный логин или пароль."))
        }
        _ => return Err(ApiError::invalid_format()),
    };

    if !state.auth.check_credentials(&login, &password) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Неверный логин или пароль."));
    }
    let Some(token) = state.auth.create_session(now_unix()) else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Вход временно недоступен.",
        ));
    };

    let secure = session::request_is_https(&headers);
    let mut response = http::json_response(StatusCode::OK, json!({ "authenticated": true }));
    attach_cookie(&mut response, session::set_cookie_header(&token, secure))?;
    Ok(response)
}

pub async fn session_delete(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let secure = session::request_is_https(&headers);
    let mut response = http::json_response(StatusCode::OK, json!({ "authenticated": false }));
    attach_cookie(&mut response, session::clear_cookie_header(secure))?;
    Ok(response)
}

// ---------------------------------------------------------------------------
// SSR internal endpoints
// ---------------------------------------------------------------------------

pub async fn internal_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let is_admin = state.auth.has_admin_session(&headers, now_unix());
    Ok(http::json_response(StatusCode::OK, json!({ "isAdmin": is_admin })))
}

pub async fn internal_geo(State(state): State<AppState>) -> ApiResult<Response> {
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::get_published_geo_data(conn, &canonical))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}

pub async fn internal_gubernia(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Response> {
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::find_published_gubernia_by_slug(conn, &canonical, &slug))
        .await?;
    match value {
        Some(value) => Ok(http::json_response(StatusCode::OK, value)),
        None => Err(ApiError::not_found("Губерния не найдена.")),
    }
}

pub async fn internal_settlement(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Response> {
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::find_published_settlement_by_slug(conn, &canonical, &slug))
        .await?;
    match value {
        Some(value) => Ok(http::json_response(StatusCode::OK, value)),
        None => Err(ApiError::not_found("Населённый пункт не найден.")),
    }
}

pub async fn internal_settlement_reference(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let is_admin = state.auth.has_admin_session(&headers, now_unix());
    let value = state
        .call(move |conn| {
            // Withdrawn settlements keep their stored reference rows, but only
            // administrators may read them once the settlement is unpublished.
            if !is_admin && !store::settlement_is_published(conn, &id)? {
                return Ok(None);
            }
            store::get_settlement_reference(conn, &id)
        })
        .await?;
    match value {
        Some(body) => Ok(http::json_response(StatusCode::OK, json!({ "body": body }))),
        None => Err(ApiError::not_found("Справка не найдена.")),
    }
}

pub async fn internal_about(State(state): State<AppState>) -> ApiResult<Response> {
    let value = state
        .call(move |conn| store::get_about_content(conn))
        .await?;
    Ok(http::json_response(StatusCode::OK, json!({ "body": value })))
}

pub async fn internal_settings(State(state): State<AppState>) -> ApiResult<Response> {
    let value = state
        .call(move |conn| store::get_settings_value(conn))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}

pub async fn internal_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if !state.auth.has_admin_session(&headers, now_unix()) {
        return Err(ApiError::unauthorized());
    }
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::get_publication_metrics(conn, &canonical))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}
