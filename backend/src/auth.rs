use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    error::{AppError, AppResult},
    security::{session_token, verify_password},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Clone, FromRow)]
struct LoginUser {
    id: Uuid,
    username: String,
    password_hash: Option<String>,
    display_name: Option<String>,
    role: String,
    status: String,
    must_change_password: bool,
    theme_preference: String,
    session_version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub role: String,
    pub must_change_password: bool,
    pub theme_preference: String,
}

impl CurrentUser {
    pub fn require_admin(&self) -> AppResult<()> {
        if self.role == "admin" {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    pub fn require_password_changed(&self) -> AppResult<()> {
        if self.role == "admin" && self.must_change_password {
            Err(AppError::Forbidden)
        } else {
            Ok(())
        }
    }
}

impl From<LoginUser> for CurrentUser {
    fn from(value: LoginUser) -> Self {
        let must_change_password = value.role == "admin" && value.must_change_password;
        Self {
            id: value.id,
            username: value.username,
            display_name: value.display_name,
            role: value.role,
            must_change_password,
            theme_preference: value.theme_preference,
        }
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        let claims = state
            .sessions
            .decode(&token)
            .map_err(|_| AppError::Unauthorized)?;
        let user = sqlx::query_as::<_, LoginUser>(
            r#"
            SELECT id, username, password_hash, display_name, role, status,
                   must_change_password, theme_preference, session_version
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await?
        .filter(|user| user.status == "active" && user.session_version == claims.session_version)
        .ok_or(AppError::Unauthorized)?;
        Ok(user.into())
    }
}

async fn login(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> AppResult<Response> {
    let username = input.username.trim();
    if username.is_empty() || input.password.is_empty() {
        return Err(AppError::Validation(
            "username and password are required".to_owned(),
        ));
    }
    let user = sqlx::query_as::<_, LoginUser>(
        r#"
        SELECT id, username, password_hash, display_name, role, status,
               must_change_password, theme_preference, session_version
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let password_hash = user.password_hash.clone().ok_or(AppError::Unauthorized)?;
    let valid = verify_password(SecretString::from(input.password), password_hash)
        .await
        .map_err(AppError::Internal)?;
    if !valid || user.status != "active" {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("UPDATE users SET last_login_at = NOW(), updated_at = NOW() WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;

    let token = state
        .sessions
        .issue(user.id, user.session_version)
        .map_err(AppError::Internal)?;
    let mut response = (StatusCode::OK, Json(CurrentUser::from(user))).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.sessions.set_cookie_header(token))
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

async fn logout(State(state): State<AppState>) -> AppResult<Response> {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.sessions.clear_cookie_header())
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

#[derive(Debug, FromRow)]
pub(crate) struct UserSecurityRow {
    pub id: Uuid,
    pub password_hash: Option<String>,
    pub session_version: i64,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct LoginAuditRow {
    last_login_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_user(role: &str, must_change_password: bool) -> CurrentUser {
        CurrentUser {
            id: Uuid::nil(),
            username: "test-user".to_owned(),
            display_name: None,
            role: role.to_owned(),
            must_change_password,
            theme_preference: "system".to_owned(),
        }
    }

    #[test]
    fn password_change_is_only_forced_for_flagged_administrators() {
        assert!(
            current_user("admin", true)
                .require_password_changed()
                .is_err()
        );
        assert!(
            current_user("admin", false)
                .require_password_changed()
                .is_ok()
        );
        assert!(
            current_user("user", true)
                .require_password_changed()
                .is_ok()
        );
    }

    #[test]
    fn ordinary_user_login_does_not_expose_admin_password_change_requirement() {
        let login_user = LoginUser {
            id: Uuid::nil(),
            username: "ordinary-user".to_owned(),
            password_hash: None,
            display_name: None,
            role: "user".to_owned(),
            status: "active".to_owned(),
            must_change_password: true,
            theme_preference: "system".to_owned(),
            session_version: 1,
        };

        assert!(!CurrentUser::from(login_user).must_change_password);
    }
}
