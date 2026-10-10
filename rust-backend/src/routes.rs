use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Map, Value};

use crate::content::MAX_POST_DOCUMENT_JSON_CHARACTERS;
use crate::error::{ApiError, ApiResult};
use crate::http;
use crate::s3::S3Storage;
use crate::session;
use crate::store;
use crate::util::{now_unix, random_id, utf16_len};
use crate::AppState;

/// `MAX_POST_REQUEST_BYTES = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 2_000`,
/// plus the 8 KiB placement-metadata margin from the post routes.
const MAX_POST_REQUEST_BYTES: usize = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 2_000;
const POST_REQUEST_BYTES: usize = MAX_POST_REQUEST_BYTES + 8_192;
/// One image upload plus the multipart envelope around it.
pub const MAX_UPLOAD_REQUEST_BYTES: usize = crate::s3::MAX_UPLOAD_BYTES + 64 * 1024;
/// Province PATCH carries a rich description document plus the slug, so it
/// keeps the same document bound as the post routes.
const GUBERNIA_REQUEST_BYTES: usize = MAX_POST_REQUEST_BYTES + 1_024;
const SETTLEMENT_REQUEST_BYTES: usize = 4_096;
const SETTINGS_REQUEST_BYTES: usize = 2_048;

fn exact_keys(payload: &Map<String, Value>, expected: &[&str]) -> bool {
    payload.len() == expected.len() && payload.keys().all(|key| expected.contains(&key.as_str()))
}

/// `true` when every submitted key is allowed; unlike [`exact_keys`] the
/// allowed fields are optional.
fn allowed_keys(payload: &Map<String, Value>, allowed: &[&str]) -> bool {
    payload.keys().all(|key| allowed.contains(&key.as_str()))
}

/// Serializes an image list for the entity responses that carry a gallery
/// next to their rich document (`{ body, images }`).
fn images_json(images: &[store::PostImageRow]) -> Value {
    Value::Array(images.iter().map(store::image_value).collect())
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
    let image_ids = payload.get("imageIds").cloned();
    let canonical = state.canonical.clone();
    let (value, detached) = state
        .call(move |conn| {
            store::update_gubernia_publication(
                conn,
                &canonical,
                &id,
                &slug,
                description.as_ref(),
                image_ids.as_ref(),
            )
        })
        .await?;
    // Images removed by this save are detached with their cleanup marker;
    // their storage objects are deleted by the background sweep.
    if detached {
        spawn_image_sweep(&state);
    }
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
    // Unpublishing deletes the province posts, so their image objects need the
    // same background cleanup as a direct post deletion.
    spawn_image_sweep(&state);
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
    let (value, detached_images) = state
        .call(move |conn| store::update_post(conn, &canonical, &id, &post_id, &payload))
        .await?;
    // A save that dropped images detached them in the same transaction; their
    // storage objects are deleted in the background, like after a deletion.
    if detached_images {
        spawn_image_sweep(&state);
    }
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
    // The post rows of its images are gone now; deleting the objects happens
    // in the background so the response does not wait for storage.
    spawn_image_sweep(&state);
    Ok(http::empty_response(StatusCode::NO_CONTENT))
}

// ---------------------------------------------------------------------------
// Browser API: post images
// ---------------------------------------------------------------------------

/// Kicks off [`crate::sweep_image_objects`] without waiting for it.
fn spawn_image_sweep(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        crate::sweep_image_objects(&state).await;
    });
}

/// Reads the single `file` multipart field. The route body cap already bounds
/// the envelope; the storage layer validates the decoded image itself.
async fn read_upload_file(multipart: &mut Multipart) -> ApiResult<axum::body::Bytes> {
    let mut file: Option<axum::body::Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad_request("Не удалось прочитать загрузку."))?
    {
        if field.name() != Some("file") || file.is_some() {
            return Err(ApiError::bad_request(
                "Загрузка должна содержать ровно одно поле «file».",
            ));
        }
        file = Some(
            field
                .bytes()
                .await
                .map_err(|_| ApiError::bad_request("Не удалось прочитать файл."))?,
        );
    }
    file.ok_or_else(|| ApiError::bad_request("Загрузка должна содержать ровно одно поле «file»."))
}

/// Keeps a failed upload tracked: the durable cleanup row is written first,
/// then the objects are deleted. Both deterministic objects gone means the
/// row can be dropped right away instead of waiting for the next sweep; if the
/// deletion or the final database step fails, the row stays behind with its
/// persisted keys and the sweep retries from them.
async fn fail_image_upload(state: &AppState, storage: &S3Storage, id: &str) {
    let requested = id.to_string();
    let row = match state
        .call(move |conn| store::mark_upload_failed(conn, &requested))
        .await
    {
        Ok(row) => row,
        Err(error) => {
            eprintln!(
                "Backend failure: cannot record the failed upload {id}: {}",
                error.message
            );
            return;
        }
    };
    let Some(row) = row else {
        return;
    };
    if let Err(error) = storage
        .delete_image(&row.original_key, &row.thumbnail_key)
        .await
    {
        eprintln!(
            "Backend failure: cannot delete the objects of the failed upload {}: {}",
            row.id, error.message
        );
        // The row keeps the deterministic keys: the sweep retries them.
        return;
    }
    let id = row.id.clone();
    if let Err(error) = state
        .call(move |conn| store::finish_image_cleanup(conn, &id))
        .await
    {
        eprintln!(
            "Backend failure: cannot drop the row of the failed upload {}: {}",
            row.id, error.message
        );
    }
}

pub async fn post_image_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let Some(storage) = state.image_storage.clone() else {
        return Err(S3Storage::unavailable());
    };
    let bytes = read_upload_file(&mut multipart).await?;
    let id = random_id();
    // The provisional row is written before any storage call, so every later
    // failure has a durable record of the deterministic keys.
    {
        let id = id.clone();
        state
            .call(move |conn| store::begin_image_upload(conn, &id))
            .await?;
    }
    let stored = match storage.put_image(&id, bytes).await {
        Ok(stored) => stored,
        Err(error) => {
            fail_image_upload(&state, &storage, &id).await;
            return Err(error);
        }
    };
    let completed = {
        let id = id.clone();
        let original_key = stored.original_key.clone();
        let thumbnail_key = stored.thumbnail_key.clone();
        let width = i64::from(stored.width);
        let height = i64::from(stored.height);
        state
            .call(move |conn| {
                store::complete_image_upload(conn, &id, &original_key, &thumbnail_key, width, height)
            })
            .await
    };
    let row = match completed {
        Ok(row) => row,
        Err(error) => {
            // Both objects exist but the row could not be completed — the
            // sweep fenced the abandoned provisional row (or it is gone):
            // roll the objects back now; a failed rollback leaves the
            // cleanup record for the sweep.
            fail_image_upload(&state, &storage, &id).await;
            return Err(error);
        }
    };
    spawn_image_sweep(&state);
    Ok(http::json_response(StatusCode::CREATED, store::image_value(&row)))
}

pub async fn post_image_get(
    State(state): State<AppState>,
    Path((id, variant)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if variant != "original" && variant != "thumbnail" {
        return Err(ApiError::not_found("Изображение не найдено."));
    }
    let access = {
        let id = id.clone();
        state
            .call(move |conn| store::load_image_access(conn, &id))
            .await?
    };
    let Some(access) = access else {
        return Err(ApiError::not_found("Изображение не найдено."));
    };
    if access.image.is_attached() {
        // Attached images follow their province — a post sits in one, a
        // province gallery and a settlement reference belong to one too:
        // only published provinces are publicly readable, everything else
        // stays invisible.
        if !access.published {
            return Err(ApiError::not_found("Изображение не найдено."));
        }
    } else {
        // Pending uploads are admin-only and only while still claimable;
        // cancelled or expired rows are cleanup matter.
        if !store::is_fresh_pending(&access.image, now_unix() * 1_000) {
            return Err(ApiError::not_found("Изображение не найдено."));
        }
        http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    }
    let Some(storage) = state.image_storage.clone() else {
        return Err(S3Storage::unavailable());
    };
    let key = if variant == "thumbnail" {
        &access.image.thumbnail_key
    } else {
        &access.image.original_key
    };
    let url = storage.presign_get(key).await?;
    let mut response = http::empty_response(StatusCode::FOUND);
    let location = HeaderValue::from_str(&url).map_err(ApiError::internal)?;
    response.headers_mut().insert(header::LOCATION, location);
    Ok(response)
}

pub async fn post_image_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    http::require_admin(&headers, &state.auth, state.frontend_origin.as_deref())?;
    let access = {
        let id = id.clone();
        state
            .call(move |conn| store::load_image_access(conn, &id))
            .await?
    };
    let Some(access) = access else {
        return Err(ApiError::not_found("Изображение не найдено."));
    };
    if access.image.is_attached() {
        // Attached images belong to their page: they leave it through the
        // form's replacement, never through the pending-upload cancel.
        return Err(ApiError::conflict(if access.image.post_id.is_some() {
            "Изображение уже привязано к публикации."
        } else {
            "Изображение привязано к странице."
        }));
    }
    let Some(storage) = state.image_storage.clone() else {
        // Without storage the objects cannot be deleted; the row must stay so
        // the keys keep pointing at them (uploads are unavailable anyway).
        return Err(S3Storage::unavailable());
    };
    let now_ms = now_unix() * 1_000;
    let image = {
        let id = id.clone();
        state
            .call(move |conn| store::take_image_for_cleanup(conn, &id, now_ms))
            .await?
    };
    let Some(image) = image else {
        return Err(ApiError::not_found("Изображение не найдено."));
    };
    // Objects first, row second: a storage error keeps the row (marked, with
    // its keys) so the sweep retries instead of leaking the objects.
    storage
        .delete_image(&image.original_key, &image.thumbnail_key)
        .await?;
    let id = image.id.clone();
    state
        .call(move |conn| store::finish_image_cleanup(conn, &id))
        .await?;
    spawn_image_sweep(&state);
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
    let detached = state
        .call(move |conn| store::delete_settlement(conn, &canonical, &id, &target))
        .await?;
    // A deleted settlement detaches its reference gallery; the storage
    // objects are deleted by the background sweep.
    if detached {
        spawn_image_sweep(&state);
    }
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
    if !allowed_keys(&payload, &["name", "body", "imageIds"])
        || (!payload.contains_key("name") && !payload.contains_key("body"))
        || (payload.contains_key("imageIds") && !payload.contains_key("body"))
    {
        return Err(ApiError::bad_request(
            "Ожидается название или документ содержимого страницы.",
        ));
    }
    let name = payload.get("name").cloned();
    let document = payload.get("body").cloned();
    let image_ids = payload.get("imageIds").cloned();
    let (name, body, images, detached) = state
        .call(move |conn| {
            let Some(settlement_id) =
                store::find_published_settlement_id_by_slug(conn, &slug)?
            else {
                return Err(ApiError::not_found("Населённый пункт не найден."));
            };
            store::save_settlement_reference(
                conn,
                &settlement_id,
                name.as_ref(),
                document.as_ref(),
                image_ids.as_ref(),
            )
        })
        .await?;
    // Images removed by this save are detached with their cleanup marker;
    // their storage objects are deleted by the background sweep.
    if detached {
        spawn_image_sweep(&state);
    }
    Ok(http::json_response(
        StatusCode::OK,
        json!({ "name": name, "body": body, "images": images_json(&images) }),
    ))
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
        Some((body, images)) => Ok(http::json_response(
            StatusCode::OK,
            json!({ "body": body, "images": images_json(&images) }),
        )),
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

pub async fn internal_metrics(State(state): State<AppState>) -> ApiResult<Response> {
    let canonical = state.canonical.clone();
    let value = state
        .call(move |conn| store::get_publication_metrics(conn, &canonical))
        .await?;
    Ok(http::json_response(StatusCode::OK, value))
}
