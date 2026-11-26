use axum::{
    Json,
    response::{IntoResponse, Response},
};
pub use openjam_core::Error;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// The API's error type: the core error plus the `/v1` response shape.
#[derive(Debug)]
pub struct ApiError(pub Error);

pub type Result<T> = std::result::Result<T, ApiError>;

impl<S: Into<Error>> From<S> for ApiError {
    fn from(error: S) -> Self {
        Self(error.into())
    }
}

impl ApiError {
    pub fn bad(message: impl Into<String>) -> Self {
        Self(Error::InvalidRequest(message.into()))
    }
}

/// The body of every failed `/v1` call. Shape kept from the source API.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiErrorBody {
    /// What went wrong and how to fix the request.
    pub error: String,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Quote it to support.
    pub request_id: Uuid,
    /// Where the error codes are documented.
    pub docs: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.0.status();
        if status >= 500 {
            tracing::error!(error = %self.0, "request failed");
        }
        let body = ApiErrorBody {
            error: self.0.public_message(),
            code: self.0.code(),
            request_id: Uuid::new_v4(),
            docs: "https://github.com/i098/openJam/blob/main/docs/api.md#errors",
        };
        let status = axum::http::StatusCode::from_u16(status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(body)).into_response()
    }
}
