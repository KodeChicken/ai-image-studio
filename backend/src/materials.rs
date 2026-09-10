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
        .route("/api/v1/material-categories", get(list_public_categories))
        .route("/api/v1/material-templates", get(list_public_templates))
        .route(
            "/api/v1/admin/material-categories",
            get(list_admin_categories).post(create_category),
        )
        .route(
            "/api/v1/admin/material-templates",
            get(list_admin_templates).post(create_template),
        )
        .route(
            "/api/v1/admin/material-templates/{id}",
            patch(update_template),
        )
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct MaterialCategory {
    id: Uuid,
    name: String,
    sort_order: i32,
    item_count: i64,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct MaterialTemplate {
    id: Uuid,
    category_id: Uuid,
    category_name: String,
    title: String,
    image_url: String,
    source_url: Option<String>,
    source_author: Option<String>,
    license: Option<String>,
    published: bool,
    sort_order: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    category_id: Option<Uuid>,
}

async fn list_public_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<MaterialCategory>>> {
    current.require_password_changed()?;
    Ok(Json(categories(&state, false).await?))
}

async fn list_admin_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<MaterialCategory>>> {
    require_admin(&current)?;
    Ok(Json(categories(&state, true).await?))
}

async fn categories(state: &AppState, include_empty: bool) -> AppResult<Vec<MaterialCategory>> {
    Ok(sqlx::query_as::<_, MaterialCategory>(
        r#"
        SELECT c.id, c.name, c.sort_order, COUNT(t.id)::BIGINT AS item_count
        FROM material_categories c
        LEFT JOIN material_templates t
          ON t.category_id = c.id AND ($1 OR t.published)
        GROUP BY c.id
        HAVING $1 OR COUNT(t.id) > 0
        ORDER BY c.sort_order, c.created_at
        "#,
    )
    .bind(include_empty)
    .fetch_all(&state.db)
    .await?)
}

async fn list_public_templates(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Vec<MaterialTemplate>>> {
    current.require_password_changed()?;
    Ok(Json(templates(&state, query.category_id, false).await?))
}

async fn list_admin_templates(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Vec<MaterialTemplate>>> {
    require_admin(&current)?;
    Ok(Json(templates(&state, query.category_id, true).await?))
}

async fn templates(
    state: &AppState,
    category_id: Option<Uuid>,
    include_unpublished: bool,
) -> AppResult<Vec<MaterialTemplate>> {
    Ok(sqlx::query_as::<_, MaterialTemplate>(
        r#"
        SELECT t.id, t.category_id, c.name AS category_name, t.title,
               COALESCE(t.image_url, '/api/v1/image-assets/' || t.image_asset_id::TEXT || '/content') AS image_url,
               t.source_url, t.source_author, t.license, t.published, t.sort_order
        FROM material_templates t
        JOIN material_categories c ON c.id = t.category_id
        WHERE ($1::UUID IS NULL OR t.category_id = $1)
          AND ($2 OR t.published)
        ORDER BY c.sort_order, t.sort_order, t.created_at
        "#,
    )
    .bind(category_id)
    .bind(include_unpublished)
    .fetch_all(&state.db)
    .await?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCategoryRequest {
    name: String,
}

async fn create_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateCategoryRequest>,
) -> AppResult<(StatusCode, Json<MaterialCategory>)> {
    require_admin(&current)?;
    let name = required(&input.name, 64, "分类名称")?;
    let sort_order = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM material_categories",
    )
    .fetch_one(&state.db)
    .await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO material_categories (name, sort_order)
        VALUES ($1, $2)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(sort_order)
    .fetch_one(&state.db)
    .await
    .map_err(map_unique)?;
    Ok((
        StatusCode::CREATED,
        Json(MaterialCategory {
            id,
            name: name.to_owned(),
            sort_order,
            item_count: 0,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTemplateRequest {
    category_id: Uuid,
    title: String,
    image_asset_id: Uuid,
    source_url: Option<String>,
    source_author: Option<String>,
    license: Option<String>,
    #[serde(default)]
    published: bool,
}

async fn create_template(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateTemplateRequest>,
) -> AppResult<(StatusCode, Json<MaterialTemplate>)> {
    require_admin(&current)?;
    let title = required(&input.title, 128, "素材名称")?;
    validate_url(input.source_url.as_deref().map(str::trim))?;
    optional(&input.source_author, 256, "来源作者")?;
    optional(&input.license, 128, "许可")?;
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM material_categories WHERE id = $1
        ) AND EXISTS (
            SELECT 1 FROM image_assets WHERE id = $2 AND owner_id = $3
        )
        "#,
    )
    .bind(input.category_id)
    .bind(input.image_asset_id)
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::Validation("分类或图片不存在".to_owned()));
    }
    let sort_order = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM material_templates WHERE category_id = $1",
    )
    .bind(input.category_id)
    .fetch_one(&state.db)
    .await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO material_templates (
            category_id, title, image_asset_id, source_url, source_author, license,
            published, sort_order, created_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(input.category_id)
    .bind(title)
    .bind(input.image_asset_id)
    .bind(trimmed(input.source_url))
    .bind(trimmed(input.source_author))
    .bind(trimmed(input.license))
    .bind(input.published)
    .bind(sort_order)
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(find_template(&state, id).await?)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTemplateRequest {
    category_id: Option<Uuid>,
    title: Option<String>,
    published: Option<bool>,
}

async fn update_template(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateTemplateRequest>,
) -> AppResult<Json<MaterialTemplate>> {
    require_admin(&current)?;
    let title = input
        .title
        .as_deref()
        .map(|value| required(value, 128, "素材名称"))
        .transpose()?;
    if let Some(category_id) = input.category_id {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM material_categories WHERE id = $1)",
        )
        .bind(category_id)
        .fetch_one(&state.db)
        .await?;
        if !exists {
            return Err(AppError::Validation("分类不存在".to_owned()));
        }
    }
    let changed = sqlx::query(
        r#"
        UPDATE material_templates
        SET category_id = COALESCE($1, category_id), title = COALESCE($2, title),
            published = COALESCE($3, published), updated_at = NOW()
        WHERE id = $4
        "#,
    )
    .bind(input.category_id)
    .bind(title)
    .bind(input.published)
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(find_template(&state, id).await?))
}

async fn find_template(state: &AppState, id: Uuid) -> AppResult<MaterialTemplate> {
    sqlx::query_as::<_, MaterialTemplate>(
        r#"
        SELECT t.id, t.category_id, c.name AS category_name, t.title,
               COALESCE(t.image_url, '/api/v1/image-assets/' || t.image_asset_id::TEXT || '/content') AS image_url,
               t.source_url, t.source_author, t.license, t.published, t.sort_order
        FROM material_templates t
        JOIN material_categories c ON c.id = t.category_id
        WHERE t.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

fn require_admin(current: &CurrentUser) -> AppResult<()> {
    current.require_admin()?;
    current.require_password_changed()
}

fn required<'a>(value: &'a str, max: usize, label: &str) -> AppResult<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max {
        return Err(AppError::Validation(format!(
            "{label}长度必须为 1 到 {max} 个字符"
        )));
    }
    Ok(value)
}

fn optional(value: &Option<String>, max: usize, label: &str) -> AppResult<()> {
    if value
        .as_deref()
        .is_some_and(|value| value.chars().count() > max)
    {
        return Err(AppError::Validation(format!(
            "{label}不能超过 {max} 个字符"
        )));
    }
    Ok(())
}

fn validate_url(value: Option<&str>) -> AppResult<()> {
    if value.is_some_and(|value| {
        value.len() > 2048 || !(value.starts_with("https://") || value.starts_with("http://"))
    }) {
        return Err(AppError::Validation(
            "来源链接必须是有效的 HTTP(S) 地址".to_owned(),
        ));
    }
    Ok(())
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn map_unique(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("23505")
    {
        AppError::Conflict("分类名称已存在".to_owned())
    } else {
        AppError::Database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_url_only_accepts_http() {
        assert!(validate_url(None).is_ok());
        assert!(validate_url(Some("https://commons.wikimedia.org/wiki/File:test.jpg")).is_ok());
        assert!(validate_url(Some("javascript:alert(1)")).is_err());
    }
}
