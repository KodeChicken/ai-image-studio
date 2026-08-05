use std::io::Cursor;

use axum::{
    Json, Router,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use chrono::{Datelike, Utc};
use image::{ImageFormat, ImageReader, imageops::FilterType};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/image-assets/uploads", post(upload))
        .route("/api/v1/image-assets/{id}", axum::routing::delete(remove))
        .route("/api/v1/image-assets/{id}/content", get(content))
}

#[derive(Clone, Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ImageAssetSummary {
    pub id: Uuid,
    pub content_url: String,
    pub mime_type: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub file_size_bytes: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct StoredAssetRow {
    pub storage_driver: String,
    pub storage_container: String,
    pub storage_key: String,
    pub mime_type: String,
    pub file_size_bytes: i64,
    pub sha256: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PendingStorageDelete {
    pub id: Uuid,
    pub storage_driver: String,
    pub storage_container: String,
    pub storage_key: String,
}

pub(crate) struct ValidatedImage {
    pub bytes: Bytes,
    pub mime_type: &'static str,
    pub extension: &'static str,
    pub width: i32,
    pub height: i32,
    pub sha256: String,
}

async fn upload(
    State(state): State<AppState>,
    current: CurrentUser,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<ImageAssetSummary>)> {
    current.require_password_changed()?;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::Validation(error.to_string()))?
        .ok_or_else(|| AppError::Validation("multipart field 'file' is required".to_owned()))?;
    if field.name() != Some("file") {
        return Err(AppError::Validation(
            "multipart field must be named 'file'".to_owned(),
        ));
    }
    let original_filename = field.file_name().map(str::to_owned);
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::Validation(error.to_string()))?;
    let max_bytes = state.settings.max_upload_size_mb * 1024 * 1024;
    if bytes.len() > max_bytes {
        return Err(AppError::Validation(format!(
            "image exceeds the {} MB upload limit",
            state.settings.max_upload_size_mb
        )));
    }
    let validated = validate_image(bytes)?;
    let asset = persist_uploaded_asset(&state, current.id, original_filename, validated).await?;
    Ok((StatusCode::CREATED, Json(asset)))
}

async fn content(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<Uuid>,
    request_headers: HeaderMap,
) -> AppResult<Response> {
    current.require_password_changed()?;
    let asset = load_owned_asset(&state, current.id, asset_id).await?;
    let etag = HeaderValue::from_str(&format!("\"{}\"", asset.sha256))
        .map_err(|error| AppError::Internal(error.into()))?;
    if request_headers.get(header::IF_NONE_MATCH) == Some(&etag) {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        set_content_cache_headers(response.headers_mut(), etag);
        return Ok(response);
    }
    let stream = state
        .storage
        .stream(
            &asset.storage_driver,
            &asset.storage_container,
            &asset.storage_key,
        )
        .await
        .map_err(|error| {
            tracing::error!(asset_id = %asset_id, error = %error, "failed to read image asset");
            AppError::NotFound
        })?;
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&asset.mime_type)
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&asset.file_size_bytes.to_string())
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    set_content_cache_headers(response.headers_mut(), etag);
    Ok(response)
}

fn set_content_cache_headers(headers: &mut HeaderMap, etag: HeaderValue) {
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(header::ETAG, etag);
}

async fn remove(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let owned = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM image_assets WHERE id = $1 AND owner_id = $2 FOR UPDATE",
    )
    .bind(asset_id)
    .bind(current.id)
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(AppError::NotFound);
    }
    let deletes = delete_unreferenced_assets(&mut tx, current.id, &[asset_id]).await?;
    if deletes.is_empty() {
        return Err(AppError::Conflict(
            "image asset is already in use".to_owned(),
        ));
    }
    tx.commit().await?;
    delete_storage_files(&state, &deletes).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn load_owned_asset(
    state: &AppState,
    owner_id: Uuid,
    asset_id: Uuid,
) -> AppResult<StoredAssetRow> {
    sqlx::query_as::<_, StoredAssetRow>(
        r#"
        SELECT storage_driver, storage_container, storage_key, mime_type, file_size_bytes, sha256
        FROM image_assets
        WHERE id = $1 AND owner_id = $2
        "#,
    )
    .bind(asset_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

pub(crate) async fn delete_unreferenced_assets(
    tx: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    candidate_ids: &[Uuid],
) -> AppResult<Vec<PendingStorageDelete>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    let assets = sqlx::query_as::<_, PendingStorageDelete>(
        r#"
        DELETE FROM image_assets a
        WHERE a.owner_id = $1 AND a.id = ANY($2)
          AND NOT EXISTS (SELECT 1 FROM message_image_assets ma WHERE ma.asset_id = a.id)
          AND NOT EXISTS (SELECT 1 FROM task_input_images ti WHERE ti.asset_id = a.id)
          AND NOT EXISTS (SELECT 1 FROM image_results ir WHERE ir.asset_id = a.id)
          AND NOT EXISTS (SELECT 1 FROM image_edit_documents d WHERE d.source_asset_id = a.id)
          AND NOT EXISTS (SELECT 1 FROM image_assets child WHERE child.parent_asset_id = a.id)
        RETURNING a.id, a.storage_driver, a.storage_container, a.storage_key
        "#,
    )
    .bind(owner_id)
    .bind(candidate_ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(assets)
}

pub(crate) async fn delete_storage_files(state: &AppState, assets: &[PendingStorageDelete]) {
    for asset in assets {
        if let Err(error) = state
            .storage
            .delete(
                &asset.storage_driver,
                &asset.storage_container,
                &asset.storage_key,
            )
            .await
        {
            tracing::error!(
                asset_id = %asset.id,
                storage_driver = %asset.storage_driver,
                storage_key = %asset.storage_key,
                error = %error,
                "asset storage delete failed; consistency cleanup must remove the orphan"
            );
        }
    }
}

pub(crate) async fn persist_asset(
    state: &AppState,
    owner_id: Uuid,
    original_filename: Option<String>,
    image: ValidatedImage,
) -> AppResult<ImageAssetSummary> {
    persist_asset_with_origin(
        state,
        owner_id,
        original_filename,
        image,
        None,
        None,
        "generated",
    )
    .await
}

async fn persist_uploaded_asset(
    state: &AppState,
    owner_id: Uuid,
    original_filename: Option<String>,
    image: ValidatedImage,
) -> AppResult<ImageAssetSummary> {
    persist_asset_with_origin(
        state,
        owner_id,
        original_filename,
        image,
        None,
        None,
        "uploaded",
    )
    .await
}

pub(crate) async fn persist_derived_asset(
    state: &AppState,
    owner_id: Uuid,
    original_filename: Option<String>,
    image: ValidatedImage,
    parent_asset_id: Uuid,
    edit_document_id: Uuid,
    origin: &'static str,
) -> AppResult<ImageAssetSummary> {
    debug_assert!(matches!(origin, "edited" | "ai_edited"));
    persist_asset_with_origin(
        state,
        owner_id,
        original_filename,
        image,
        Some(parent_asset_id),
        Some(edit_document_id),
        origin,
    )
    .await
}

async fn persist_asset_with_origin(
    state: &AppState,
    owner_id: Uuid,
    original_filename: Option<String>,
    image: ValidatedImage,
    parent_asset_id: Option<Uuid>,
    edit_document_id: Option<Uuid>,
    origin: &'static str,
) -> AppResult<ImageAssetSummary> {
    let now = Utc::now();
    let asset_id = Uuid::new_v4();
    let storage_key = format!(
        "{}/{:02}/{}/{asset_id}.{}",
        now.year(),
        now.month(),
        owner_id,
        image.extension
    );
    let stored = state
        .storage
        .put(&storage_key, image.bytes.clone())
        .await
        .map_err(|error| AppError::Internal(error.context("failed to persist image file")))?;
    let insert = sqlx::query(
        r#"
        INSERT INTO image_assets (
            id, owner_id, storage_driver, storage_container, storage_key,
            original_filename, mime_type, width, height, file_size_bytes, sha256,
            parent_asset_id, edit_document_id, asset_origin
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        "#,
    )
    .bind(asset_id)
    .bind(owner_id)
    .bind(stored.driver)
    .bind(&stored.container)
    .bind(&stored.key)
    .bind(original_filename)
    .bind(image.mime_type)
    .bind(image.width)
    .bind(image.height)
    .bind(image.bytes.len() as i64)
    .bind(image.sha256)
    .bind(parent_asset_id)
    .bind(edit_document_id)
    .bind(origin)
    .execute(&state.db)
    .await;
    if let Err(error) = insert {
        if let Err(cleanup_error) = state
            .storage
            .delete(stored.driver, &stored.container, &stored.key)
            .await
        {
            tracing::error!(error = %cleanup_error, storage_key = %stored.key, "asset compensation delete failed");
        }
        return Err(AppError::Database(error));
    }
    Ok(ImageAssetSummary {
        id: asset_id,
        content_url: format!("/api/v1/image-assets/{asset_id}/content"),
        mime_type: image.mime_type.to_owned(),
        width: Some(image.width),
        height: Some(image.height),
        file_size_bytes: image.bytes.len() as i64,
    })
}

pub(crate) fn validate_image(bytes: Bytes) -> AppResult<ValidatedImage> {
    if bytes.is_empty() {
        return Err(AppError::Validation("image file is empty".to_owned()));
    }
    let format = image::guess_format(&bytes)
        .map_err(|_| AppError::Validation("unsupported or invalid image format".to_owned()))?;
    let (mime_type, extension) = match format {
        image::ImageFormat::Png => ("image/png", "png"),
        image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        image::ImageFormat::WebP => ("image/webp", "webp"),
        _ => {
            return Err(AppError::Validation(
                "only PNG, JPEG and WebP images are supported".to_owned(),
            ));
        }
    };
    let reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| AppError::Validation("image data is corrupted".to_owned()))?;
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return Err(AppError::Validation(
            "image dimensions are outside the supported range".to_owned(),
        ));
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(ValidatedImage {
        bytes,
        mime_type,
        extension,
        width: width as i32,
        height: height as i32,
        sha256,
    })
}

pub(crate) fn resize_exact(
    image: ValidatedImage,
    width: u32,
    height: u32,
) -> AppResult<ValidatedImage> {
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return Err(AppError::Validation(
            "target image dimensions are outside the supported range".to_owned(),
        ));
    }
    if image.width == width as i32 && image.height == height as i32 {
        return Ok(image);
    }

    let format = match image.extension {
        "png" => ImageFormat::Png,
        "jpg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::WebP,
        _ => {
            return Err(AppError::Validation(
                "unsupported image format for resizing".to_owned(),
            ));
        }
    };
    let decoded = image::load_from_memory_with_format(&image.bytes, format)
        .map_err(|_| AppError::Validation("image data is corrupted".to_owned()))?;
    let resized = decoded.resize_to_fill(width, height, FilterType::Lanczos3);
    let mut output = Cursor::new(Vec::new());
    resized
        .write_to(&mut output, format)
        .map_err(|error| AppError::Internal(error.into()))?;
    validate_image(Bytes::from(output.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizes_and_center_crops_to_the_exact_dimensions() {
        let source = image::DynamicImage::new_rgb8(8, 4);
        let mut encoded = Cursor::new(Vec::new());
        source.write_to(&mut encoded, ImageFormat::Png).unwrap();
        let validated = validate_image(Bytes::from(encoded.into_inner())).unwrap();

        let resized = resize_exact(validated, 16, 16).unwrap();

        assert_eq!((resized.width, resized.height), (16, 16));
        assert_eq!(resized.mime_type, "image/png");
    }
}
