use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use rand::{Rng, distributions::Alphanumeric};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    AppState, Settings,
    auth::{CurrentUser, UserSecurityRow},
    error::{AppError, AppResult},
    security::{hash_password, verify_password},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/users/me", get(me))
        .route("/api/v1/users/me/preferences", patch(update_preferences))
        .route("/api/v1/users/me/change-password", post(change_password))
        .route("/api/v1/admin/users", get(list_users).post(create_user))
        .route("/api/v1/admin/users/{id}", patch(update_user))
        .route(
            "/api/v1/admin/users/{id}/reset-password",
            post(reset_password),
        )
}

pub async fn bootstrap_admin(db: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    if !settings.bootstrap_admin_enabled {
        return Ok(());
    }
    let admin_exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE role = 'admin')")
            .fetch_one(db)
            .await?;
    if admin_exists {
        return Ok(());
    }

    let password_hash = hash_password(settings.bootstrap_admin_password.clone()).await?;
    let inserted = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, must_change_password)
        VALUES ($1, $2, 'Administrator', 'admin', $3)
        ON CONFLICT (username) DO NOTHING
        "#,
    )
    .bind(&settings.bootstrap_admin_username)
    .bind(password_hash)
    .bind(settings.bootstrap_admin_force_password_change)
    .execute(db)
    .await?
    .rows_affected();
    if inserted == 1 {
        tracing::info!(username = %settings.bootstrap_admin_username, "bootstrap administrator created");
        return Ok(());
    }
    let role = sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE username = $1")
        .bind(&settings.bootstrap_admin_username)
        .fetch_optional(db)
        .await?;
    if role.as_deref() != Some("admin") {
        anyhow::bail!(
            "bootstrap administrator username '{}' is already used by a non-admin account",
            settings.bootstrap_admin_username
        );
    }
    Ok(())
}

async fn me(current: CurrentUser) -> Json<CurrentUser> {
    Json(current)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesRequest {
    theme_preference: String,
}

async fn update_preferences(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<PreferencesRequest>,
) -> AppResult<Json<CurrentUser>> {
    if !matches!(input.theme_preference.as_str(), "light" | "dark" | "system") {
        return Err(AppError::Validation("invalid theme preference".to_owned()));
    }
    sqlx::query("UPDATE users SET theme_preference = $1, updated_at = NOW() WHERE id = $2")
        .bind(&input.theme_preference)
        .bind(current.id)
        .execute(&state.db)
        .await?;
    Ok(Json(CurrentUser {
        theme_preference: input.theme_preference,
        ..current
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<ChangePasswordRequest>,
) -> AppResult<Response> {
    validate_password(&input.new_password)?;
    let security = sqlx::query_as::<_, UserSecurityRow>(
        "SELECT id, password_hash, session_version FROM users WHERE id = $1",
    )
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    let current_hash = security.password_hash.ok_or(AppError::Unauthorized)?;
    if !verify_password(SecretString::from(input.current_password), current_hash)
        .await
        .map_err(AppError::Internal)?
    {
        return Err(AppError::Unauthorized);
    }

    let new_hash = hash_password(SecretString::from(input.new_password))
        .await
        .map_err(AppError::Internal)?;
    let new_version = security.session_version + 1;
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = $1, must_change_password = FALSE,
            session_version = $2, updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(new_hash)
    .bind(new_version)
    .bind(security.id)
    .execute(&state.db)
    .await?;

    let token = state
        .sessions
        .issue(current.id, new_version)
        .map_err(AppError::Internal)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.sessions.set_cookie_header(token))
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

#[derive(Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct AdminUserSummary {
    id: Uuid,
    username: String,
    display_name: Option<String>,
    role: String,
    status: String,
    must_change_password: bool,
    theme_preference: String,
    provider_count: i64,
    task_count: i64,
    image_bytes: i64,
    last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_users(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<AdminUserSummary>>> {
    current.require_admin()?;
    current.require_password_changed()?;
    let users = sqlx::query_as::<_, AdminUserSummary>(
        r#"
        SELECT u.id, u.username, u.display_name, u.role, u.status,
               u.must_change_password, u.theme_preference, u.last_login_at, u.created_at,
               COUNT(DISTINCT p.id)::BIGINT AS provider_count,
               COUNT(DISTINCT t.id)::BIGINT AS task_count,
               COALESCE(MAX(asset_stats.image_bytes), 0)::BIGINT AS image_bytes
        FROM users u
        LEFT JOIN providers p ON p.owner_id = u.id AND p.deleted_at IS NULL
        LEFT JOIN image_tasks t ON t.user_id = u.id
        LEFT JOIN (
            SELECT owner_id, SUM(file_size_bytes)::BIGINT AS image_bytes
            FROM image_assets GROUP BY owner_id
        ) asset_stats ON asset_stats.owner_id = u.id
        GROUP BY u.id
        ORDER BY u.created_at DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateUserRequest {
    username: String,
    display_name: Option<String>,
    role: String,
    password: String,
}

async fn create_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateUserRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    current.require_admin()?;
    current.require_password_changed()?;
    validate_username(&input.username)?;
    validate_role(&input.role)?;
    validate_password(&input.password)?;
    let hash = hash_password(SecretString::from(input.password))
        .await
        .map_err(AppError::Internal)?;
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO users (username, display_name, role, password_hash, must_change_password)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(input.username.trim())
    .bind(input.display_name)
    .bind(&input.role)
    .bind(hash)
    .bind(input.role == "admin")
    .fetch_one(&state.db)
    .await
    .map_err(map_unique_conflict)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(Deserialize)]
struct UpdateUserRequest {
    role: Option<String>,
    status: Option<String>,
}

async fn update_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(user_id): Path<Uuid>,
    Json(input): Json<UpdateUserRequest>,
) -> AppResult<StatusCode> {
    current.require_admin()?;
    current.require_password_changed()?;
    if user_id == current.id && input.status.as_deref() == Some("disabled") {
        return Err(AppError::Validation(
            "cannot disable the current administrator".to_owned(),
        ));
    }
    if let Some(role) = input.role.as_deref() {
        validate_role(role)?;
    }
    if let Some(status) = input.status.as_deref()
        && !matches!(status, "active" | "disabled")
    {
        return Err(AppError::Validation("invalid user status".to_owned()));
    }
    let changed = sqlx::query(
        r#"
        UPDATE users
        SET role = COALESCE($1, role), status = COALESCE($2, status),
            must_change_password = CASE
                WHEN $1 = 'user' THEN FALSE
                ELSE must_change_password
            END,
            session_version = CASE WHEN $2 = 'disabled' THEN session_version + 1 ELSE session_version END,
            updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(input.role)
    .bind(input.status)
    .bind(user_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_password(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    current.require_admin()?;
    current.require_password_changed()?;
    let temporary: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    let hash = hash_password(SecretString::from(temporary.clone()))
        .await
        .map_err(AppError::Internal)?;
    let changed = sqlx::query(
        r#"
        UPDATE users
        SET password_hash = $1, must_change_password = (role = 'admin'),
            session_version = session_version + 1, updated_at = NOW()
        WHERE id = $2
        "#,
    )
    .bind(hash)
    .bind(user_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "temporaryPassword": temporary })))
}

fn validate_username(username: &str) -> AppResult<()> {
    if username.len() < 3 || username.len() > 64 {
        return Err(AppError::Validation(
            "username must contain 3 to 64 characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> AppResult<()> {
    if password.len() < 8 {
        return Err(AppError::Validation(
            "password must contain at least 8 characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> AppResult<()> {
    if matches!(role, "admin" | "user") {
        Ok(())
    } else {
        Err(AppError::Validation("invalid user role".to_owned()))
    }
}

fn map_unique_conflict(error: sqlx::Error) -> AppError {
    if matches!(&error, sqlx::Error::Database(db_error) if db_error.is_unique_violation()) {
        AppError::Conflict("username already exists".to_owned())
    } else {
        AppError::Database(error)
    }
}
