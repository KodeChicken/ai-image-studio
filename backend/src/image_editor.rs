use axum::{
    Json, Router,
    extract::{Multipart, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
    images::{self, ImageAssetSummary},
    tasks,
};

const MIN_EDGE: i32 = 16;
const MAX_EDGE: i32 = 8192;
const MAX_PIXELS: i64 = 33_554_432;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/image-edit-documents", post(create))
        .route("/api/v1/image-edit-documents/{id}", get(find).put(update))
        .route("/api/v1/image-edit-documents/{id}/exports", post(export))
        .route(
            "/api/v1/image-edit-documents/{id}/ai-expand",
            post(ai_expand),
        )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditorDocumentV1 {
    schema_version: i32,
    canvas: EditorCanvas,
    layout: EditorLayout,
    image: EditorImage,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditorCanvas {
    width: i32,
    height: i32,
    background: EditorBackground,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
enum EditorBackground {
    Transparent,
    Color { color: String },
    BlurredImage { blur_radius: f64 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditorLayout {
    fit_strategy: String,
    anchor: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditorImage {
    asset_id: Uuid,
    x: f64,
    y: f64,
    scale_x: f64,
    scale_y: f64,
    rotation: f64,
    flip_x: bool,
    flip_y: bool,
    crop: EditorCrop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EditorCrop {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Debug, FromRow)]
struct AssetRow {
    id: Uuid,
    mime_type: String,
    width: Option<i32>,
    height: Option<i32>,
    file_size_bytes: i64,
}

impl AssetRow {
    fn summary(&self) -> AppResult<ImageAssetSummary> {
        if self.width.is_none() || self.height.is_none() {
            return Err(AppError::Validation(
                "image asset does not contain decoded dimensions".to_owned(),
            ));
        }
        Ok(ImageAssetSummary {
            id: self.id,
            content_url: format!("/api/v1/image-assets/{}/content", self.id),
            mime_type: self.mime_type.clone(),
            width: self.width,
            height: self.height,
            file_size_bytes: self.file_size_bytes,
        })
    }
}

#[derive(Debug, FromRow)]
struct DocumentRow {
    id: Uuid,
    source_asset_id: Uuid,
    title: String,
    schema_version: i32,
    document_json: Value,
    version: i64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentView {
    id: Uuid,
    source_asset_id: Uuid,
    title: String,
    schema_version: i32,
    version: i64,
    document: EditorDocumentV1,
    source_asset: ImageAssetSummary,
    image_asset: ImageAssetSummary,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    source_asset_id: Uuid,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "canvas".to_owned()
}

async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateRequest>,
) -> AppResult<(StatusCode, Json<DocumentView>)> {
    current.require_password_changed()?;
    if !matches!(input.mode.as_str(), "crop" | "canvas" | "expand") {
        return Err(AppError::Validation(
            "mode must be crop, canvas or expand".to_owned(),
        ));
    }
    let asset = load_asset(&state, current.id, input.source_asset_id).await?;
    let width = asset.width.ok_or_else(|| {
        AppError::Validation("source image does not contain decoded dimensions".to_owned())
    })?;
    let height = asset.height.ok_or_else(|| {
        AppError::Validation("source image does not contain decoded dimensions".to_owned())
    })?;
    validate_canvas_size(width, height)?;
    let document = default_document(input.source_asset_id, width, height, &input.mode);
    let document_json =
        serde_json::to_value(&document).map_err(|error| AppError::Internal(error.into()))?;
    let id = Uuid::new_v4();
    let title = "图片成品";
    sqlx::query(
        r#"
        INSERT INTO image_edit_documents (
            id, owner_id, source_asset_id, title, schema_version, document_json
        ) VALUES ($1, $2, $3, $4, 1, $5)
        "#,
    )
    .bind(id)
    .bind(current.id)
    .bind(input.source_asset_id)
    .bind(title)
    .bind(document_json)
    .execute(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(load_view(&state, current.id, id).await?),
    ))
}

async fn find(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(document_id): Path<Uuid>,
) -> AppResult<Json<DocumentView>> {
    current.require_password_changed()?;
    Ok(Json(load_view(&state, current.id, document_id).await?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    version: i64,
    schema_version: i32,
    document: Value,
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(document_id): Path<Uuid>,
    Json(input): Json<UpdateRequest>,
) -> AppResult<Json<DocumentView>> {
    current.require_password_changed()?;
    if input.schema_version != 1 {
        return Err(AppError::Validation(
            "only editor schema version 1 is supported".to_owned(),
        ));
    }
    let document = parse_document(input.document)?;
    validate_document(&state, current.id, &document).await?;
    let changed = sqlx::query(
        r#"
        UPDATE image_edit_documents
        SET schema_version = $1, document_json = $2,
            version = version + 1, updated_at = NOW()
        WHERE id = $3 AND owner_id = $4 AND version = $5
        "#,
    )
    .bind(input.schema_version)
    .bind(serde_json::to_value(document).map_err(|error| AppError::Internal(error.into()))?)
    .bind(document_id)
    .bind(current.id)
    .bind(input.version)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM image_edit_documents WHERE id = $1 AND owner_id = $2)",
        )
        .bind(document_id)
        .bind(current.id)
        .fetch_one(&state.db)
        .await?;
        return Err(if exists {
            AppError::Conflict("editor document was updated in another page".to_owned())
        } else {
            AppError::NotFound
        });
    }
    Ok(Json(load_view(&state, current.id, document_id).await?))
}

#[derive(Default)]
struct ExportFields {
    file: Option<(Option<String>, Bytes)>,
    document_version: Option<i64>,
    format: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
}

async fn export(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(document_id): Path<Uuid>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<ImageAssetSummary>)> {
    current.require_password_changed()?;
    let mut fields = ExportFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::Validation(error.to_string()))?
    {
        match field.name() {
            Some("file") => {
                let filename = field.file_name().map(str::to_owned);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|error| AppError::Validation(error.to_string()))?;
                fields.file = Some((filename, bytes));
            }
            Some("documentVersion") => fields.document_version = Some(parse_field(field).await?),
            Some("format") => fields.format = Some(field.text().await.map_err(field_error)?),
            Some("width") => fields.width = Some(parse_field(field).await?),
            Some("height") => fields.height = Some(parse_field(field).await?),
            _ => {}
        }
    }
    let (filename, bytes) = fields
        .file
        .ok_or_else(|| AppError::Validation("multipart file is required".to_owned()))?;
    let declared_version = fields
        .document_version
        .ok_or_else(|| AppError::Validation("documentVersion is required".to_owned()))?;
    let format = fields
        .format
        .ok_or_else(|| AppError::Validation("format is required".to_owned()))?;
    let declared_width = fields
        .width
        .ok_or_else(|| AppError::Validation("width is required".to_owned()))?;
    let declared_height = fields
        .height
        .ok_or_else(|| AppError::Validation("height is required".to_owned()))?;
    let view = load_view(&state, current.id, document_id).await?;
    if declared_version != view.version {
        return Err(AppError::Conflict(
            "export is based on a stale editor document version".to_owned(),
        ));
    }
    if declared_width != view.document.canvas.width
        || declared_height != view.document.canvas.height
    {
        return Err(AppError::Validation(
            "declared export dimensions do not match the editor document".to_owned(),
        ));
    }
    if bytes.len() > state.settings.max_upload_size_mb * 1024 * 1024 {
        return Err(AppError::Validation(format!(
            "image exceeds the {} MB upload limit",
            state.settings.max_upload_size_mb
        )));
    }
    let validated = images::validate_image(bytes)?;
    validate_export(&validated, &format, declared_width, declared_height)?;
    let asset = images::persist_derived_asset(
        &state,
        current.id,
        filename,
        validated,
        view.document.image.asset_id,
        document_id,
        "edited",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(asset)))
}

async fn parse_field<T: std::str::FromStr>(
    field: axum::extract::multipart::Field<'_>,
) -> AppResult<T> {
    field
        .text()
        .await
        .map_err(field_error)?
        .parse()
        .map_err(|_| AppError::Validation("multipart numeric field is invalid".to_owned()))
}

fn field_error(error: axum::extract::multipart::MultipartError) -> AppError {
    AppError::Validation(error.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiExpandRequest {
    provider_id: Uuid,
    model_id: Uuid,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    parameters: Map<String, Value>,
}

async fn ai_expand(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(document_id): Path<Uuid>,
    Json(input): Json<AiExpandRequest>,
) -> AppResult<(StatusCode, Json<tasks::EditorTaskCreated>)> {
    current.require_password_changed()?;
    let view = load_view(&state, current.id, document_id).await?;
    let capabilities = sqlx::query_scalar::<_, Value>(
        r#"
        SELECT m.capabilities
        FROM models m JOIN providers p ON p.id = m.provider_id
        WHERE m.id = $1 AND m.provider_id = $2 AND p.owner_id = $3
          AND m.enabled AND p.enabled AND m.deleted_at IS NULL AND p.deleted_at IS NULL
        "#,
    )
    .bind(input.model_id)
    .bind(input.provider_id)
    .bind(current.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if !supports_outpaint(&capabilities) {
        return Err(AppError::Validation(
            "the selected model has not declared AI outpaint support".to_owned(),
        ));
    }
    if !supports_input_mime(&capabilities, &view.image_asset.mime_type) {
        return Err(AppError::Validation(
            "the selected model does not support the current image format".to_owned(),
        ));
    }
    let created = tasks::create_editor_task(
        &state,
        current.id,
        tasks::NewEditorTaskRequest {
            edit_document_id: document_id,
            content: input.prompt.trim().to_owned(),
            provider_id: input.provider_id,
            model_id: input.model_id,
            parameters: Value::Object(input.parameters),
            input_asset_ids: vec![view.document.image.asset_id],
        },
    )
    .await?;
    tasks::dispatch_processing(state, created.task_id).await?;
    Ok((StatusCode::ACCEPTED, Json(created)))
}

fn supports_outpaint(capabilities: &Value) -> bool {
    capabilities
        .get("image_edit_capability")
        .and_then(|value| value.get("supportsOutpaint"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn supports_input_mime(capabilities: &Value, mime_type: &str) -> bool {
    capabilities
        .get("image_edit_capability")
        .and_then(|value| value.get("supportedInputMimeTypes"))
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(mime_type)))
}

async fn load_view(state: &AppState, owner_id: Uuid, document_id: Uuid) -> AppResult<DocumentView> {
    let row = sqlx::query_as::<_, DocumentRow>(
        r#"
        SELECT id, source_asset_id, title, schema_version, document_json,
               version, created_at, updated_at
        FROM image_edit_documents
        WHERE id = $1 AND owner_id = $2
        "#,
    )
    .bind(document_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let document = parse_document(row.document_json)?;
    let source_asset = load_asset(state, owner_id, row.source_asset_id).await?;
    let image_asset = load_asset(state, owner_id, document.image.asset_id).await?;
    validate_document_geometry(
        &document,
        image_asset.width.unwrap_or(0),
        image_asset.height.unwrap_or(0),
    )?;
    Ok(DocumentView {
        id: row.id,
        source_asset_id: row.source_asset_id,
        title: row.title,
        schema_version: row.schema_version,
        version: row.version,
        document,
        source_asset: source_asset.summary()?,
        image_asset: image_asset.summary()?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn load_asset(state: &AppState, owner_id: Uuid, asset_id: Uuid) -> AppResult<AssetRow> {
    sqlx::query_as::<_, AssetRow>(
        r#"
        SELECT id, mime_type, width, height, file_size_bytes
        FROM image_assets WHERE id = $1 AND owner_id = $2
        "#,
    )
    .bind(asset_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

async fn validate_document(
    state: &AppState,
    owner_id: Uuid,
    document: &EditorDocumentV1,
) -> AppResult<()> {
    let asset = load_asset(state, owner_id, document.image.asset_id).await?;
    validate_document_geometry(
        document,
        asset.width.unwrap_or(0),
        asset.height.unwrap_or(0),
    )
}

fn parse_document(value: Value) -> AppResult<EditorDocumentV1> {
    serde_json::from_value(value).map_err(|_| {
        AppError::Validation("editor document does not match schema version 1".to_owned())
    })
}

fn validate_document_geometry(
    document: &EditorDocumentV1,
    source_width: i32,
    source_height: i32,
) -> AppResult<()> {
    if document.schema_version != 1 {
        return Err(AppError::Validation(
            "only editor schema version 1 is supported".to_owned(),
        ));
    }
    validate_canvas_size(document.canvas.width, document.canvas.height)?;
    if !matches!(
        document.layout.fit_strategy.as_str(),
        "cover" | "contain" | "free" | "stretch"
    ) || !matches!(
        document.layout.anchor.as_str(),
        "top-left"
            | "top"
            | "top-right"
            | "left"
            | "center"
            | "right"
            | "bottom-left"
            | "bottom"
            | "bottom-right"
    ) {
        return Err(AppError::Validation(
            "editor layout strategy or anchor is invalid".to_owned(),
        ));
    }
    let crop = &document.image.crop;
    if source_width <= 0
        || source_height <= 0
        || ![crop.x, crop.y, crop.width, crop.height]
            .into_iter()
            .all(f64::is_finite)
        || crop.x < 0.0
        || crop.y < 0.0
        || crop.x.fract() != 0.0
        || crop.y.fract() != 0.0
        || crop.width.fract() != 0.0
        || crop.height.fract() != 0.0
        || crop.width < f64::from(MIN_EDGE)
        || crop.height < f64::from(MIN_EDGE)
        || crop.x + crop.width > f64::from(source_width)
        || crop.y + crop.height > f64::from(source_height)
    {
        return Err(AppError::Validation(
            "crop rectangle is outside the source image".to_owned(),
        ));
    }
    let image = &document.image;
    if ![
        image.x,
        image.y,
        image.scale_x,
        image.scale_y,
        image.rotation,
    ]
    .into_iter()
    .all(f64::is_finite)
        || image.scale_x <= 0.0
        || image.scale_y <= 0.0
    {
        return Err(AppError::Validation(
            "image transform is invalid".to_owned(),
        ));
    }
    match &document.canvas.background {
        EditorBackground::Color { color }
            if color.is_empty() || color.len() > 64 || !color.starts_with('#') =>
        {
            return Err(AppError::Validation(
                "background color is invalid".to_owned(),
            ));
        }
        EditorBackground::BlurredImage { blur_radius }
            if !blur_radius.is_finite() || !(0.0..=100.0).contains(blur_radius) =>
        {
            return Err(AppError::Validation(
                "background blur radius is invalid".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_canvas_size(width: i32, height: i32) -> AppResult<()> {
    if width < MIN_EDGE || height < MIN_EDGE || width > MAX_EDGE || height > MAX_EDGE {
        return Err(AppError::Validation(
            "editor dimensions must be between 16 and 8192 pixels".to_owned(),
        ));
    }
    if i64::from(width) * i64::from(height) > MAX_PIXELS {
        return Err(AppError::Validation(
            "editor canvas exceeds the 33,554,432 pixel limit".to_owned(),
        ));
    }
    Ok(())
}

fn validate_export(
    image: &images::ValidatedImage,
    format: &str,
    width: i32,
    height: i32,
) -> AppResult<()> {
    let expected_mime = match format {
        "png" => "image/png",
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => {
            return Err(AppError::Validation(
                "format must be png, jpeg or webp".to_owned(),
            ));
        }
    };
    if image.mime_type != expected_mime || image.width != width || image.height != height {
        return Err(AppError::Validation(
            "decoded export format or dimensions do not match the declaration".to_owned(),
        ));
    }
    Ok(())
}

fn default_document(asset_id: Uuid, width: i32, height: i32, mode: &str) -> EditorDocumentV1 {
    EditorDocumentV1 {
        schema_version: 1,
        canvas: EditorCanvas {
            width,
            height,
            background: EditorBackground::Transparent,
        },
        layout: EditorLayout {
            fit_strategy: if mode == "crop" { "free" } else { "cover" }.to_owned(),
            anchor: "center".to_owned(),
        },
        image: EditorImage {
            asset_id,
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            crop: EditorCrop {
                x: 0.0,
                y: 0.0,
                width: f64::from(width),
                height: f64::from(height),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_4k_crop_height_is_valid() {
        let mut document = default_document(Uuid::new_v4(), 4096, 4096, "crop");
        document.image.crop.height = 261.0;
        document.canvas.height = 261;
        validate_document_geometry(&document, 4096, 4096).unwrap();
    }

    #[test]
    fn invalid_crop_and_oversized_canvas_are_rejected() {
        let mut document = default_document(Uuid::new_v4(), 1024, 1024, "canvas");
        document.image.crop.x = 900.0;
        assert!(validate_document_geometry(&document, 1024, 1024).is_err());
        assert!(validate_canvas_size(8192, 8192).is_err());
    }

    #[test]
    fn outpaint_requires_an_explicit_capability() {
        assert!(!supports_outpaint(
            &serde_json::json!({ "image_edit": true })
        ));
        assert!(supports_outpaint(&serde_json::json!({
            "image_edit_capability": {
                "supportsOutpaint": true,
                "supportedInputMimeTypes": ["image/png"]
            }
        })));
        assert!(supports_input_mime(
            &serde_json::json!({
                "image_edit_capability": { "supportedInputMimeTypes": ["image/png"] }
            }),
            "image/png"
        ));
    }

    #[test]
    fn export_validation_checks_real_format_and_dimensions() {
        for (format, image_format) in [
            ("png", image::ImageFormat::Png),
            ("jpeg", image::ImageFormat::Jpeg),
            ("webp", image::ImageFormat::WebP),
        ] {
            let source = image::DynamicImage::new_rgb8(1920, 1080);
            let mut encoded = std::io::Cursor::new(Vec::new());
            source.write_to(&mut encoded, image_format).unwrap();
            let validated = images::validate_image(Bytes::from(encoded.into_inner())).unwrap();
            validate_export(&validated, format, 1920, 1080).unwrap();
            assert!(validate_export(&validated, format, 1080, 1920).is_err());
        }
    }
}
