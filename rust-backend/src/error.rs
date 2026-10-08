use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Mirrors the TypeScript error envelopes: `{"error": message}` with the
/// status recorded on `AdminRequestError` / `GuberniaPublicationError`, always
/// under `Cache-Control: no-store`. Unexpected failures log and fall back to
/// the generic 500 message.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Требуется вход администратора.")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Запрос отклонён.")
    }

    pub fn unsupported_media_type() -> Self {
        Self::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Ожидается JSON-запрос.")
    }

    pub fn body_too_large() -> Self {
        Self::bad_request("Тело запроса слишком большое.")
    }

    pub fn invalid_format() -> Self {
        Self::bad_request("Неверный формат запроса.")
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("Backend failure: {error}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Внутренняя ошибка сервера.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.message });
        let mut response = (self.status, axum::Json(body)).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        Self::internal(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(error)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
