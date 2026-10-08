use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::error::{ApiError, ApiResult};
use crate::session::{self, AuthConfig};
use crate::util::{js_number, now_unix};

/// Requests without an explicit per-route cap are still bounded at 16 MiB;
/// every real payload is far smaller and the routes that matter keep their
/// exact TypeScript byte caps.
pub const DEFAULT_BODY_LIMIT: usize = 16 * 1024 * 1024;

/// `assertAdminMutation`: session first (401), then the Origin/host defence
/// (403) for browser-initiated writes.
pub fn require_admin(
    headers: &HeaderMap,
    auth: &AuthConfig,
    frontend_origin: Option<&str>,
) -> ApiResult<()> {
    if !auth.has_admin_session(headers, now_unix()) {
        return Err(ApiError::unauthorized());
    }
    if !session::origin_allowed(headers, frontend_origin) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// `readJsonObject`: JSON content type first (415), then a bounded read (400
/// "too large"), then JSON parse and object shape (400 "invalid format").
pub async fn read_json_object(
    headers: &HeaderMap,
    body: Body,
    max_bytes: Option<usize>,
) -> ApiResult<Map<String, Value>> {
    let value = read_json_value(headers, body, max_bytes, true).await?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(ApiError::invalid_format()),
    }
}

/// `request.json()` semantics for the session route: no content-type check.
pub async fn read_json_any(headers: &HeaderMap, body: Body) -> ApiResult<Value> {
    read_json_value(headers, body, None, false).await
}

async fn read_json_value(
    headers: &HeaderMap,
    body: Body,
    max_bytes: Option<usize>,
    require_json_content_type: bool,
) -> ApiResult<Value> {
    if require_json_content_type {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .map(str::to_ascii_lowercase);
        let matches = content_type.as_deref().is_some_and(|value| {
            value.starts_with("application/json") && {
                let rest = value["application/json".len()..].trim_start();
                rest.is_empty() || rest.starts_with(';')
            }
        });
        if !matches {
            return Err(ApiError::unsupported_media_type());
        }
    }

    if let (Some(max), Some(declared)) = (
        max_bytes,
        headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()),
    ) {
        let number = js_number(declared);
        if number.is_finite() && number > max as f64 {
            return Err(ApiError::body_too_large());
        }
    }

    let limit = max_bytes.map(|max| max + 1).unwrap_or(DEFAULT_BODY_LIMIT);
    let bytes = to_bytes(body, limit).await.map_err(|_| match max_bytes {
        Some(_) => ApiError::body_too_large(),
        None => ApiError::invalid_format(),
    })?;
    if let Some(max) = max_bytes {
        if bytes.len() > max {
            return Err(ApiError::body_too_large());
        }
    }

    let source = String::from_utf8_lossy(&bytes);
    serde_json::from_str(&source).map_err(|_| ApiError::invalid_format())
}

pub fn json_response(status: StatusCode, value: Value) -> Response {
    let mut response = (status, axum::Json(value)).into_response();
    no_store(&mut response);
    response
}

pub fn empty_response(status: StatusCode) -> Response {
    let mut response = status.into_response();
    no_store(&mut response);
    response
}

pub fn no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

/// Every `/api` and `/internal` response bypasses caches, including errors.
pub async fn no_store_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    no_store(&mut response);
    response
}
