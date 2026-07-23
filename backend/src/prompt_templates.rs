use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/prompt-templates", get(list).post(create))
        .route("/api/v1/prompt-templates/{id}", patch(update))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct PromptTemplate {
    id: Uuid,
    owner_id: Option<Uuid>,
    template_type: String,
    title: String,
    prompt: String,
    negative_prompt: Option<String>,
    tags: Vec<String>,
    is_public: bool,
    enabled: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(alias = "template_type")]
    template_type: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Vec<PromptTemplate>>> {
    current.require_password_changed()?;
    if let Some(value) = query.template_type.as_deref() {
        validate_type(value)?;
    }
    let templates = sqlx::query_as::<_, PromptTemplate>(
        r#"
        SELECT id, owner_id, template_type, title, prompt, negative_prompt, tags,
               is_public, enabled, created_at, updated_at
        FROM prompt_templates
        WHERE (owner_id = $1 OR (owner_id IS NULL AND is_public))
          AND ($2::VARCHAR IS NULL OR template_type = $2)
          AND enabled
        ORDER BY owner_id NULLS FIRST, created_at
        "#,
    )
    .bind(current.id)
    .bind(query.template_type)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(templates))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    #[serde(default = "default_type")]
    template_type: String,
    title: String,
    prompt: String,
    negative_prompt: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateRequest>,
) -> AppResult<(StatusCode, Json<PromptTemplate>)> {
    current.require_password_changed()?;
    validate_type(&input.template_type)?;
    validate_text(&input.title, 1, 256, "title")?;
    validate_text(&input.prompt, 1, 8000, "prompt")?;
    if input.tags.len() > 20 {
        return Err(AppError::Validation(
            "a template can contain at most 20 tags".to_owned(),
        ));
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO prompt_templates (
            owner_id, template_type, title, prompt, negative_prompt, tags
        ) VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(current.id)
    .bind(input.template_type)
    .bind(input.title.trim())
    .bind(input.prompt.trim())
    .bind(input.negative_prompt)
    .bind(input.tags)
    .fetch_one(&state.db)
    .await?;
    let template = find_owned(&state, current.id, id).await?;
    Ok((StatusCode::CREATED, Json(template)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    title: Option<String>,
    prompt: Option<String>,
    negative_prompt: Option<String>,
    tags: Option<Vec<String>>,
    enabled: Option<bool>,
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateRequest>,
) -> AppResult<Json<PromptTemplate>> {
    current.require_password_changed()?;
    if let Some(value) = input.title.as_deref() {
        validate_text(value, 1, 256, "title")?;
    }
    if let Some(value) = input.prompt.as_deref() {
        validate_text(value, 1, 8000, "prompt")?;
    }
    if input.tags.as_ref().is_some_and(|tags| tags.len() > 20) {
        return Err(AppError::Validation(
            "a template can contain at most 20 tags".to_owned(),
        ));
    }
    let changed = sqlx::query(
        r#"
        UPDATE prompt_templates
        SET title = COALESCE($1, title), prompt = COALESCE($2, prompt),
            negative_prompt = COALESCE($3, negative_prompt), tags = COALESCE($4, tags),
            enabled = COALESCE($5, enabled), updated_at = NOW()
        WHERE id = $6 AND owner_id = $7
        "#,
    )
    .bind(input.title.map(|value| value.trim().to_owned()))
    .bind(input.prompt.map(|value| value.trim().to_owned()))
    .bind(input.negative_prompt)
    .bind(input.tags)
    .bind(input.enabled)
    .bind(id)
    .bind(current.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(find_owned(&state, current.id, id).await?))
}

async fn find_owned(state: &AppState, owner_id: Uuid, id: Uuid) -> AppResult<PromptTemplate> {
    sqlx::query_as::<_, PromptTemplate>(
        r#"
        SELECT id, owner_id, template_type, title, prompt, negative_prompt, tags,
               is_public, enabled, created_at, updated_at
        FROM prompt_templates WHERE id = $1 AND owner_id = $2
        "#,
    )
    .bind(id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

fn default_type() -> String {
    "general".to_owned()
}

fn validate_type(value: &str) -> AppResult<()> {
    if matches!(value, "general" | "style") {
        Ok(())
    } else {
        Err(AppError::Validation("invalid template type".to_owned()))
    }
}

fn validate_text(value: &str, min: usize, max: usize, field: &str) -> AppResult<()> {
    if (min..=max).contains(&value.trim().len()) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{field} must contain {min} to {max} characters"
        )))
    }
}
