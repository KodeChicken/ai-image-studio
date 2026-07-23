use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("authentication required")]
    Unauthorized,
    #[error("administrator permission required")]
    Forbidden,
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("upstream provider request failed: {0}")]
    Upstream(String),
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, public_message) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", self.to_string()),
            Self::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN", self.to_string()),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", self.to_string()),
            Self::Validation(_) => (
                StatusCode::BAD_REQUEST,
                "VALIDATION_ERROR",
                self.to_string(),
            ),
            Self::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT", self.to_string()),
            Self::Upstream(_) => (StatusCode::BAD_GATEWAY, "PROVIDER_ERROR", self.to_string()),
            Self::Database(_) | Self::Internal(_) => {
                tracing::error!(error = %self, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "internal server error".to_owned(),
                )
            }
        };
        (
            status,
            Json(ErrorBody {
                error: ErrorDetail {
                    code,
                    message: public_message,
                },
            }),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
