use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
    provider_adapters, providers,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/models", get(list))
        .route("/api/v1/providers/{id}/models/discover", post(discover))
        .route(
            "/api/v1/providers/{provider_id}/test-generation",
            post(test_generation),
        )
        .route(
            "/api/v1/providers/{provider_id}/models/{model_id}",
            patch(update).post(verify),
        )
        .route(
            "/api/v1/providers/{provider_id}/models/{model_id}/pricing",
            get(list_pricing).post(create_pricing),
        )
        .route(
            "/api/v1/providers/{provider_id}/models/{model_id}/pricing/{pricing_id}",
            axum::routing::delete(remove_pricing),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateModelRequest {
    enabled: Option<bool>,
    display_name: Option<String>,
    capabilities: Option<Value>,
    parameter_schema: Option<Value>,
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((provider_id, model_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<UpdateModelRequest>,
) -> AppResult<Json<ImageModel>> {
    current.require_password_changed()?;
    if input
        .display_name
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value.len() > 128)
    {
        return Err(AppError::Validation(
            "displayName must contain 1 to 128 characters".to_owned(),
        ));
    }
    if input
        .capabilities
        .as_ref()
        .is_some_and(|value| !value.is_object())
        || input
            .parameter_schema
            .as_ref()
            .is_some_and(|value| !value.is_object())
    {
        return Err(AppError::Validation(
            "capabilities and parameterSchema must be JSON objects".to_owned(),
        ));
    }
    let manual_override = input.capabilities.is_some() || input.parameter_schema.is_some();
    let changed = sqlx::query(
        r#"
        UPDATE models m
        SET enabled = COALESCE($1, m.enabled),
            display_name = COALESCE($2, m.display_name),
            capabilities = COALESCE($3, m.capabilities),
            parameter_schema = COALESCE($4, m.parameter_schema),
            capability_source = CASE WHEN $5 THEN 'manual_override' ELSE m.capability_source END,
            availability_status = CASE WHEN $5 THEN 'verified' ELSE m.availability_status END,
            last_verified_at = CASE WHEN $5 THEN NOW() ELSE m.last_verified_at END,
            updated_at = NOW()
        FROM providers p
        WHERE m.id = $6 AND m.provider_id = $7
          AND p.id = m.provider_id AND p.owner_id = $8
          AND m.deleted_at IS NULL AND p.deleted_at IS NULL
        "#,
    )
    .bind(input.enabled)
    .bind(input.display_name.map(|value| value.trim().to_owned()))
    .bind(input.capabilities)
    .bind(input.parameter_schema)
    .bind(manual_override)
    .bind(model_id)
    .bind(provider_id)
    .bind(current.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let model = sqlx::query_as::<_, ImageModel>(
        r#"
        SELECT m.id, m.provider_id, p.provider_type, m.model_key, m.upstream_model_id,
               m.display_name, m.capabilities, m.parameter_schema,
               m.availability_status, m.discovery_source, m.capability_source,
               m.last_discovered_at, m.last_verified_at, m.enabled
        FROM models m JOIN providers p ON p.id = m.provider_id
        WHERE m.id = $1 AND m.provider_id = $2 AND p.owner_id = $3
        "#,
    )
    .bind(model_id)
    .bind(provider_id)
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(model))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ImageModel {
    id: Uuid,
    provider_id: Uuid,
    provider_type: String,
    model_key: String,
    upstream_model_id: String,
    display_name: String,
    capabilities: Value,
    parameter_schema: Value,
    availability_status: String,
    discovery_source: String,
    capability_source: String,
    last_discovered_at: Option<chrono::DateTime<chrono::Utc>>,
    last_verified_at: Option<chrono::DateTime<chrono::Utc>>,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    provider_id: Option<Uuid>,
    #[serde(default)]
    include_discovered: bool,
    #[serde(default)]
    image_only: bool,
}

async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Vec<ImageModel>>> {
    current.require_password_changed()?;
    let models = sqlx::query_as::<_, ImageModel>(
        r#"
        SELECT m.id, m.provider_id, p.provider_type, m.model_key, m.upstream_model_id,
               m.display_name, m.capabilities, m.parameter_schema,
               m.availability_status, m.discovery_source, m.capability_source,
               m.last_discovered_at, m.last_verified_at, m.enabled
        FROM models m
        JOIN providers p ON p.id = m.provider_id
        WHERE p.owner_id = $1 AND p.deleted_at IS NULL AND m.deleted_at IS NULL
          AND ($2::UUID IS NULL OR m.provider_id = $2)
          AND ($3 OR m.availability_status = 'verified')
          AND (NOT $4 OR m.capabilities @> '{"text_to_image": true}'::JSONB)
        ORDER BY m.sort_order, m.display_name
        "#,
    )
    .bind(current.id)
    .bind(query.provider_id)
    .bind(query.include_discovered)
    .bind(query.image_only)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(models))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryResult {
    discovered: usize,
    verified_image_models: usize,
    models: Vec<ImageModel>,
}

async fn discover(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(provider_id): Path<Uuid>,
) -> AppResult<Json<DiscoveryResult>> {
    current.require_password_changed()?;
    let provider = providers::credential_row(&state, current.id, provider_id).await?;
    if !matches!(
        provider.provider_type.as_str(),
        "openai-compatible" | "gemini" | "grok"
    ) {
        return Err(AppError::Validation(
            "model discovery is not implemented for this provider type".to_owned(),
        ));
    }
    let (ciphertext, nonce) = provider
        .credential_ciphertext
        .zip(provider.credential_nonce)
        .ok_or_else(|| AppError::Validation("provider API key is not configured".to_owned()))?;
    let credential = state
        .credential_cipher
        .decrypt(&ciphertext, &nonce)
        .map_err(AppError::Internal)?;
    let upstream = match provider_adapters::list_models(
        &provider.provider_type,
        &state.http_client,
        &provider.base_url,
        &credential,
    )
    .await
    {
        Ok(models) => models,
        Err(error) => {
            let summary = error.to_string();
            providers::update_health(&state, provider.id, "unhealthy", Some(&summary)).await?;
            return Err(AppError::Upstream(summary));
        }
    };

    let mut unique = std::collections::BTreeMap::new();
    for model in upstream {
        let id = model.id.trim();
        if !id.is_empty() {
            unique
                .entry(id.to_owned())
                .or_insert((model.display_name, model.metadata));
        }
    }
    let mut verified = 0;
    let refresh_started_at = chrono::Utc::now();
    let mut tx = state.db.begin().await?;
    for (model_id, (display_name, metadata)) in &unique {
        let catalog = catalog_for(&provider.provider_type, model_id);
        if catalog.verified {
            verified += 1;
        }
        sqlx::query(
            r#"
            INSERT INTO models (
                provider_id, model_key, upstream_model_id, display_name,
                capabilities, parameter_schema, availability_status,
                discovery_source, capability_source, upstream_metadata,
                last_discovered_at, last_verified_at
            )
            VALUES ($1, $2, $2, $3, $4, $5, $6, 'upstream_list', $7, $8, NOW(), $9)
            ON CONFLICT (provider_id, upstream_model_id) WHERE deleted_at IS NULL
            DO UPDATE SET
                upstream_metadata = EXCLUDED.upstream_metadata,
                last_discovered_at = NOW(),
                availability_status = CASE
                    WHEN models.capability_source = 'manual_override' THEN models.availability_status
                    ELSE EXCLUDED.availability_status
                END,
                capabilities = CASE
                    WHEN models.capability_source = 'manual_override' THEN models.capabilities
                    ELSE EXCLUDED.capabilities
                END,
                parameter_schema = CASE
                    WHEN models.capability_source = 'manual_override' THEN models.parameter_schema
                    ELSE EXCLUDED.parameter_schema
                END,
                capability_source = CASE
                    WHEN models.capability_source = 'manual_override' THEN models.capability_source
                    ELSE EXCLUDED.capability_source
                END,
                last_verified_at = CASE
                    WHEN EXCLUDED.availability_status = 'verified' THEN NOW()
                    ELSE models.last_verified_at
                END,
                updated_at = NOW()
            "#,
        )
        .bind(provider.id)
        .bind(model_id)
        .bind(display_name.as_deref().unwrap_or(model_id))
        .bind(catalog.capabilities)
        .bind(catalog.parameters)
        .bind(if catalog.verified { "verified" } else { "discovered" })
        .bind(if catalog.verified { "official_catalog" } else { "probe" })
        .bind(Value::Object(metadata.clone()))
        .bind(if catalog.verified { Some(chrono::Utc::now()) } else { None })
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        UPDATE models
        SET availability_status = 'unavailable', updated_at = NOW()
        WHERE provider_id = $1 AND deleted_at IS NULL
          AND (last_discovered_at IS NULL OR last_discovered_at < $2)
        "#,
    )
    .bind(provider.id)
    .bind(refresh_started_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    providers::update_health(&state, provider.id, "healthy", None).await?;

    let models = sqlx::query_as::<_, ImageModel>(
        r#"
        SELECT m.id, m.provider_id, p.provider_type, m.model_key, m.upstream_model_id,
               m.display_name, m.capabilities, m.parameter_schema,
               m.availability_status, m.discovery_source, m.capability_source,
               m.last_discovered_at, m.last_verified_at, m.enabled
        FROM models m JOIN providers p ON p.id = m.provider_id
        WHERE m.provider_id = $1 AND p.owner_id = $2 AND m.deleted_at IS NULL
        ORDER BY m.sort_order, m.display_name
        "#,
    )
    .bind(provider.id)
    .bind(current.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(DiscoveryResult {
        discovered: unique.len(),
        verified_image_models: verified,
        models,
    }))
}

#[derive(FromRow)]
struct VerificationTarget {
    upstream_model_id: String,
    model_key: String,
    display_name: String,
    capabilities: Value,
    parameter_schema: Value,
}

async fn verify(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((provider_id, model_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<ImageModel>> {
    current.require_password_changed()?;
    run_model_test(
        &state,
        &current,
        provider_id,
        model_id,
        "A simple blue circle centered on a plain white background.".to_owned(),
        json!({ "n": 1 }),
        false,
        "provider.models.verify",
    )
    .await?;
    find_model(&state, current.id, provider_id, model_id).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestGenerationRequest {
    prompt: String,
    #[serde(default = "empty_object")]
    parameters: Value,
    model_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestGenerationResponse {
    model_id: Uuid,
    model_name: String,
    image_data_url: String,
    mime_type: String,
    width: i32,
    height: i32,
    latency_ms: u64,
}

async fn test_generation(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(provider_id): Path<Uuid>,
    Json(input): Json<TestGenerationRequest>,
) -> AppResult<Json<TestGenerationResponse>> {
    current.require_password_changed()?;
    let prompt = input.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > 4000 {
        return Err(AppError::Validation(
            "prompt must contain 1 to 4000 characters".to_owned(),
        ));
    }
    let mut parameters = input.parameters;
    let parameter_object = parameters
        .as_object_mut()
        .ok_or_else(|| AppError::Validation("parameters must be a JSON object".to_owned()))?;
    parameter_object.insert("n".to_owned(), json!(1));
    let result = run_model_test(
        &state,
        &current,
        provider_id,
        input.model_id,
        prompt.to_owned(),
        parameters,
        true,
        "provider.models.test-generation",
    )
    .await?;
    let image_data_url = format!(
        "data:{};base64,{}",
        result.image.mime_type,
        STANDARD.encode(&result.image.bytes)
    );
    Ok(Json(TestGenerationResponse {
        model_id: input.model_id,
        model_name: result.model_name,
        image_data_url,
        mime_type: result.image.mime_type.to_owned(),
        width: result.image.width,
        height: result.image.height,
        latency_ms: result.latency_ms,
    }))
}

struct ModelTestResult {
    model_name: String,
    image: crate::images::ValidatedImage,
    latency_ms: u64,
}

#[allow(clippy::too_many_arguments)]
async fn run_model_test(
    state: &AppState,
    current: &CurrentUser,
    provider_id: Uuid,
    model_id: Uuid,
    prompt: String,
    parameters: Value,
    require_known_image_capability: bool,
    log_route: &str,
) -> AppResult<ModelTestResult> {
    let target = sqlx::query_as::<_, VerificationTarget>(
        r#"
        SELECT m.upstream_model_id, m.model_key, m.display_name,
               m.capabilities, m.parameter_schema
        FROM models m
        JOIN providers p ON p.id = m.provider_id
        WHERE m.id = $1 AND m.provider_id = $2 AND p.owner_id = $3
          AND m.enabled AND m.deleted_at IS NULL
          AND p.enabled AND p.deleted_at IS NULL
        "#,
    )
    .bind(model_id)
    .bind(provider_id)
    .bind(current.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if require_known_image_capability
        && target
            .capabilities
            .get("text_to_image")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(AppError::Validation(
            "selected model is not classified as an image generation model".to_owned(),
        ));
    }
    if require_known_image_capability {
        crate::tasks::validate_task_parameters(
            &parameters,
            &target.parameter_schema,
            "generation",
        )?;
    }
    let provider = providers::credential_row(state, current.id, provider_id).await?;
    let (ciphertext, nonce) = provider
        .credential_ciphertext
        .zip(provider.credential_nonce)
        .ok_or_else(|| AppError::Validation("provider API key is not configured".to_owned()))?;
    let credential = state
        .credential_cipher
        .decrypt(&ciphertext, &nonce)
        .map_err(AppError::Internal)?;
    let trace_id = Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let verification = async {
        let images = provider_adapters::create_images(
            &provider.provider_type,
            "generation",
            &state.http_client,
            &provider.base_url,
            &credential,
            provider_adapters::ProviderRequest {
                model: target.upstream_model_id.clone(),
                prompt,
                parameters,
                inputs: Vec::new(),
                max_image_bytes: state.settings.max_provider_image_size_mb * 1024 * 1024,
            },
        )
        .await?;
        let image = images
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("provider returned no image"))?;
        let bytes = crate::tasks::provider_image_bytes(state, image).await?;
        crate::images::validate_image(bytes).map_err(|error| anyhow::anyhow!(error.to_string()))
    }
    .await;
    record_verification_request(
        state,
        log_route,
        &trace_id,
        &provider.provider_type,
        &target.model_key,
        started.elapsed(),
        verification.as_ref().err().map(ToString::to_string),
    )
    .await;
    let image = match verification {
        Ok(image) => image,
        Err(error) => {
            sqlx::query(
                "UPDATE models SET availability_status = 'unavailable', updated_at = NOW() WHERE id = $1 AND provider_id = $2",
            )
            .bind(model_id)
            .bind(provider_id)
            .execute(&state.db)
            .await?;
            providers::update_health(state, provider_id, "unhealthy", Some(&error.to_string()))
                .await?;
            return Err(AppError::Upstream(error.to_string()));
        }
    };
    sqlx::query(
        r#"
        UPDATE models
        SET availability_status = 'verified', capability_source = 'probe',
            capabilities = capabilities || '{"text_to_image": true}'::JSONB,
            last_verified_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND provider_id = $2
        "#,
    )
    .bind(model_id)
    .bind(provider_id)
    .execute(&state.db)
    .await?;
    providers::update_health(state, provider_id, "healthy", None).await?;
    Ok(ModelTestResult {
        model_name: target.display_name,
        image,
        latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

async fn record_verification_request(
    state: &AppState,
    route: &str,
    trace_id: &str,
    provider_type: &str,
    model_key: &str,
    elapsed: std::time::Duration,
    error: Option<String>,
) {
    let failed = error.is_some();
    let result = sqlx::query(
        r#"
        INSERT INTO request_logs (
            trace_id, route, method, provider_type, model_key,
            status_code, latency_ms, error_code, error_summary
        ) VALUES ($1, $2, 'POST', $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(trace_id)
    .bind(route)
    .bind(provider_type)
    .bind(model_key)
    .bind(if failed { 502 } else { 200 })
    .bind(elapsed.as_millis().min(i64::MAX as u128) as i64)
    .bind(failed.then_some("UPSTREAM_ERROR"))
    .bind(error.map(|value| value.chars().take(1000).collect::<String>()))
    .execute(&state.db)
    .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, "failed to persist model verification request log");
    }
}

async fn find_model(
    state: &AppState,
    owner_id: Uuid,
    provider_id: Uuid,
    model_id: Uuid,
) -> AppResult<Json<ImageModel>> {
    let model = sqlx::query_as::<_, ImageModel>(
        r#"
        SELECT m.id, m.provider_id, p.provider_type, m.model_key, m.upstream_model_id,
               m.display_name, m.capabilities, m.parameter_schema,
               m.availability_status, m.discovery_source, m.capability_source,
               m.last_discovered_at, m.last_verified_at, m.enabled
        FROM models m JOIN providers p ON p.id = m.provider_id
        WHERE m.id = $1 AND m.provider_id = $2 AND p.owner_id = $3
          AND m.deleted_at IS NULL AND p.deleted_at IS NULL
        "#,
    )
    .bind(model_id)
    .bind(provider_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(model))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct ModelPrice {
    id: Uuid,
    model_id: Uuid,
    pricing_type: String,
    dimension_key: String,
    price: String,
    currency: String,
    effective_from: chrono::DateTime<chrono::Utc>,
    effective_to: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_pricing(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((provider_id, model_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<Vec<ModelPrice>>> {
    current.require_password_changed()?;
    ensure_model_owned(&state, current.id, provider_id, model_id).await?;
    let prices = sqlx::query_as::<_, ModelPrice>(
        r#"
        SELECT id, model_id, pricing_type, dimension_key, price::TEXT AS price,
               currency, effective_from, effective_to, created_at
        FROM model_pricing
        WHERE model_id = $1
        ORDER BY effective_from DESC
        "#,
    )
    .bind(model_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(prices))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePricingRequest {
    price: String,
    #[serde(default = "default_currency")]
    currency: String,
    effective_from: Option<chrono::DateTime<chrono::Utc>>,
    effective_to: Option<chrono::DateTime<chrono::Utc>>,
}

async fn create_pricing(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((provider_id, model_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<CreatePricingRequest>,
) -> AppResult<(StatusCode, Json<ModelPrice>)> {
    current.require_password_changed()?;
    current.require_admin()?;
    ensure_model_owned(&state, current.id, provider_id, model_id).await?;
    let price = validate_price(&input.price)?;
    let currency = validate_currency(&input.currency)?;
    let effective_from = input.effective_from.unwrap_or_else(chrono::Utc::now);
    if input
        .effective_to
        .is_some_and(|effective_to| effective_to <= effective_from)
    {
        return Err(AppError::Validation(
            "effectiveTo must be later than effectiveFrom".to_owned(),
        ));
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO model_pricing (
            model_id, pricing_type, dimension_key, price, currency,
            effective_from, effective_to
        ) VALUES ($1, 'image', 'image', $2::NUMERIC, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(model_id)
    .bind(price)
    .bind(currency)
    .bind(effective_from)
    .bind(input.effective_to)
    .fetch_one(&state.db)
    .await
    .map_err(map_pricing_conflict)?;
    let price = find_price(&state, current.id, provider_id, model_id, id).await?;
    Ok((StatusCode::CREATED, Json(price)))
}

async fn remove_pricing(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((provider_id, model_id, pricing_id)): Path<(Uuid, Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    current.require_admin()?;
    let changed = sqlx::query(
        r#"
        DELETE FROM model_pricing mp
        USING models m, providers p
        WHERE mp.id = $1 AND mp.model_id = $2
          AND m.id = mp.model_id AND m.provider_id = $3
          AND p.id = m.provider_id AND p.owner_id = $4
        "#,
    )
    .bind(pricing_id)
    .bind(model_id)
    .bind(provider_id)
    .bind(current.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn find_price(
    state: &AppState,
    owner_id: Uuid,
    provider_id: Uuid,
    model_id: Uuid,
    pricing_id: Uuid,
) -> AppResult<ModelPrice> {
    sqlx::query_as::<_, ModelPrice>(
        r#"
        SELECT mp.id, mp.model_id, mp.pricing_type, mp.dimension_key,
               mp.price::TEXT AS price, mp.currency, mp.effective_from,
               mp.effective_to, mp.created_at
        FROM model_pricing mp
        JOIN models m ON m.id = mp.model_id
        JOIN providers p ON p.id = m.provider_id
        WHERE mp.id = $1 AND mp.model_id = $2 AND m.provider_id = $3
          AND p.owner_id = $4
        "#,
    )
    .bind(pricing_id)
    .bind(model_id)
    .bind(provider_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

async fn ensure_model_owned(
    state: &AppState,
    owner_id: Uuid,
    provider_id: Uuid,
    model_id: Uuid,
) -> AppResult<()> {
    let owned = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM models m
            JOIN providers p ON p.id = m.provider_id
            WHERE m.id = $1 AND m.provider_id = $2 AND p.owner_id = $3
              AND m.deleted_at IS NULL AND p.deleted_at IS NULL
        )
        "#,
    )
    .bind(model_id)
    .bind(provider_id)
    .bind(owner_id)
    .fetch_one(&state.db)
    .await?;
    if owned {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

fn validate_price(value: &str) -> AppResult<String> {
    let value = value.trim();
    let valid_shape = !value.is_empty()
        && value.len() <= 25
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && value.bytes().filter(|byte| *byte == b'.').count() <= 1;
    let valid_value = value
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite() && number >= 0.0);
    if valid_shape && valid_value {
        Ok(value.to_owned())
    } else {
        Err(AppError::Validation(
            "price must be a non-negative decimal number".to_owned(),
        ))
    }
}

fn validate_currency(value: &str) -> AppResult<String> {
    let currency = value.trim().to_ascii_uppercase();
    if (3..=16).contains(&currency.len())
        && currency.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        Ok(currency)
    } else {
        Err(AppError::Validation(
            "currency must contain 3 to 16 letters or digits".to_owned(),
        ))
    }
}

fn default_currency() -> String {
    "USD".to_owned()
}

fn empty_object() -> Value {
    json!({})
}

fn map_pricing_conflict(error: sqlx::Error) -> AppError {
    if matches!(&error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23P01"))
    {
        AppError::Conflict("pricing effective period overlaps an existing price".to_owned())
    } else {
        AppError::Database(error)
    }
}

struct CatalogEntry {
    verified: bool,
    capabilities: Value,
    parameters: Value,
}

fn catalog_for(provider_type: &str, model_id: &str) -> CatalogEntry {
    let id = model_id.to_ascii_lowercase();
    if provider_type == "gemini"
        && id.starts_with("gemini-")
        && (id.contains("image") || id.contains("banana"))
    {
        return CatalogEntry {
            verified: true,
            capabilities: json!({
                "text_to_image": true,
                "image_edit": true,
                "reference_image": true,
                "multiple_reference_images": true,
                "sizes": ["auto", "1k", "2k", "4k"],
                "aspect_ratios": ["auto", "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9"],
                "max_images_per_request": 1,
                "output_formats": ["png", "jpeg", "webp"],
                "native_streaming": false,
                "native_multi_turn": true,
                "image_edit_capability": {
                    "supportsImageEdit": true,
                    "supportsMask": false,
                    "supportsOutpaint": false,
                    "supportedInputMimeTypes": ["image/png", "image/jpeg", "image/webp"],
                    "supportedOutputSizes": ["auto", "1k", "2k", "4k"],
                    "maxInputImages": 1,
                    "maxDimension": 4096
                }
            }),
            parameters: json!({
                "meta": { "source": "official_catalog", "modelFamily": "gemini-image", "schemaVersion": "1" },
                "parameters": {
                    "aspect_ratio": { "type": "enum", "default": "auto", "options": ["auto", "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9"] },
                    "size": { "type": "enum", "default": "auto", "options": ["auto", "1k", "2k", "4k"] },
                    "n": { "type": "integer", "default": 1, "min": 1, "max": 1 }
                }
            }),
        };
    }
    if provider_type == "grok" && (id.contains("image") || id.contains("imagine")) {
        return CatalogEntry {
            verified: true,
            capabilities: json!({
                "text_to_image": true,
                "image_edit": true,
                "reference_image": true,
                "multiple_reference_images": false,
                "sizes": ["auto"],
                "aspect_ratios": ["auto", "1:1", "3:2", "2:3", "16:9", "9:16"],
                "max_images_per_request": 10,
                "output_formats": ["png", "jpeg"],
                "native_streaming": false,
                "native_multi_turn": false,
                "image_edit_capability": {
                    "supportsImageEdit": true,
                    "supportsMask": false,
                    "supportsOutpaint": false,
                    "supportedInputMimeTypes": ["image/png", "image/jpeg"],
                    "supportedOutputSizes": ["auto"],
                    "maxInputImages": 10,
                    "maxDimension": null
                }
            }),
            parameters: json!({
                "meta": { "source": "official_catalog", "modelFamily": "grok-image", "schemaVersion": "1" },
                "parameters": {
                    "aspect_ratio": { "type": "enum", "default": "auto", "options": ["auto", "1:1", "3:2", "2:3", "16:9", "9:16"] },
                    "size": { "type": "enum", "default": "auto", "options": ["auto"] },
                    "resolution": { "type": "enum", "default": "1k", "options": ["1k", "2k"] },
                    "n": { "type": "integer", "default": 1, "min": 1, "max": 10 },
                    "response_format": { "type": "enum", "default": "url", "options": ["url", "b64_json"] }
                }
            }),
        };
    }
    if id.starts_with("gpt-image-") {
        let is_gpt_image_2 = id == "gpt-image-2" || id.starts_with("gpt-image-2-");
        let sizes = if is_gpt_image_2 {
            json!([
                "auto",
                "1024x1024",
                "1536x1024",
                "1024x1536",
                "2048x2048",
                "2048x1152",
                "1152x2048",
                "3840x2160",
                "2160x3840"
            ])
        } else {
            json!(["auto", "1024x1024", "1536x1024", "1024x1536"])
        };
        let aspect_ratios = if is_gpt_image_2 {
            json!(["auto", "1:1", "3:2", "2:3", "16:9", "9:16"])
        } else {
            json!(["auto", "1:1", "3:2", "2:3"])
        };
        return CatalogEntry {
            verified: true,
            capabilities: json!({
                "text_to_image": true,
                "image_edit": true,
                "reference_image": true,
                "multiple_reference_images": true,
                "sizes": sizes.clone(),
                "aspect_ratios": aspect_ratios,
                "max_images_per_request": 10,
                "output_formats": ["png", "jpeg", "webp"],
                "quality_levels": ["auto", "low", "medium", "high"],
                "supports_transparent_background": !is_gpt_image_2,
                "native_streaming": true,
                "max_partial_images": 3,
                "native_multi_turn": false,
                "image_edit_capability": {
                    "supportsImageEdit": true,
                    "supportsMask": true,
                    "supportsOutpaint": true,
                    "supportedInputMimeTypes": ["image/png", "image/jpeg", "image/webp"],
                    "supportedOutputSizes": if is_gpt_image_2 { json!("custom") } else { sizes },
                    "maxInputImages": 10,
                    "maxDimension": if is_gpt_image_2 { 3840 } else { 1536 }
                }
            }),
            parameters: gpt_image_parameters(is_gpt_image_2),
        };
    }
    if id == "dall-e-3" {
        return CatalogEntry {
            verified: true,
            capabilities: json!({
                "text_to_image": true,
                "image_edit": false,
                "reference_image": false,
                "sizes": ["auto", "1024x1024", "1792x1024", "1024x1792"],
                "aspect_ratios": ["auto", "1:1", "16:9", "9:16"],
                "max_images_per_request": 1,
                "output_formats": ["url", "b64_json"],
                "quality_levels": ["standard", "hd"],
                "supports_transparent_background": false,
                "native_streaming": false,
                "max_partial_images": 0,
                "native_multi_turn": false,
                "image_edit_capability": {
                    "supportsImageEdit": false,
                    "supportsMask": false,
                    "supportsOutpaint": false,
                    "supportedInputMimeTypes": [],
                    "supportedOutputSizes": ["auto", "1024x1024", "1792x1024", "1024x1792"],
                    "maxInputImages": 0,
                    "maxDimension": 1792
                }
            }),
            parameters: json!({
                "meta": { "source": "official_catalog", "modelFamily": "dall-e", "schemaVersion": "1" },
                "parameters": {
                    "aspect_ratio": { "type": "enum", "default": "auto", "options": ["auto", "1:1", "16:9", "9:16"] },
                    "size": { "type": "enum", "default": "auto", "options": ["auto", "1024x1024", "1792x1024", "1024x1792"] },
                    "quality": { "type": "enum", "default": "standard", "options": ["standard", "hd"] },
                    "n": { "type": "integer", "default": 1, "min": 1, "max": 1 },
                    "response_format": { "type": "enum", "default": "b64_json", "options": ["b64_json", "url"] },
                    "style": { "type": "enum", "default": "vivid", "options": ["vivid", "natural"] }
                }
            }),
        };
    }
    if id == "dall-e-2" {
        return CatalogEntry {
            verified: true,
            capabilities: json!({
                "text_to_image": true,
                "image_edit": true,
                "reference_image": true,
                "multiple_reference_images": false,
                "sizes": ["auto", "256x256", "512x512", "1024x1024"],
                "aspect_ratios": ["auto", "1:1"],
                "max_images_per_request": 10,
                "output_formats": ["url", "b64_json"],
                "quality_levels": ["standard"],
                "supports_transparent_background": false,
                "native_streaming": false,
                "max_partial_images": 0,
                "native_multi_turn": false,
                "image_edit_capability": {
                    "supportsImageEdit": true,
                    "supportsMask": true,
                    "supportsOutpaint": true,
                    "supportedInputMimeTypes": ["image/png"],
                    "supportedOutputSizes": ["auto", "256x256", "512x512", "1024x1024"],
                    "maxInputImages": 1,
                    "maxDimension": 1024
                }
            }),
            parameters: json!({
                "meta": { "source": "official_catalog", "modelFamily": "dall-e-2", "schemaVersion": "2026-07-21" },
                "parameters": {
                    "aspect_ratio": { "type": "enum", "default": "auto", "options": ["auto", "1:1"] },
                    "size": { "type": "enum", "default": "auto", "options": ["auto", "256x256", "512x512", "1024x1024"] },
                    "quality": { "type": "enum", "default": "standard", "options": ["standard"] },
                    "n": { "type": "integer", "default": 1, "min": 1, "max": 10 },
                    "response_format": { "type": "enum", "default": "b64_json", "options": ["b64_json", "url"] }
                }
            }),
        };
    }
    CatalogEntry {
        verified: false,
        capabilities: json!({}),
        parameters: json!({
            "meta": { "source": "unclassified", "modelFamily": "unknown", "schemaVersion": "1" },
            "parameters": {}
        }),
    }
}

fn gpt_image_parameters(is_gpt_image_2: bool) -> Value {
    let sizes = if is_gpt_image_2 {
        json!([
            "auto",
            "1024x1024",
            "1536x1024",
            "1024x1536",
            "2048x2048",
            "2048x1152",
            "1152x2048",
            "3840x2160",
            "2160x3840"
        ])
    } else {
        json!(["auto", "1024x1024", "1536x1024", "1024x1536"])
    };
    let aspect_ratios = if is_gpt_image_2 {
        json!(["auto", "1:1", "3:2", "2:3", "16:9", "9:16"])
    } else {
        json!(["auto", "1:1", "3:2", "2:3"])
    };
    let size_definition = if is_gpt_image_2 {
        json!({
            "type": "enum",
            "default": "auto",
            "options": sizes,
            "allow_custom": true,
            "constraints": {
                "edgeMultiple": 16,
                "maxEdge": 3840,
                "minPixels": 655360,
                "maxPixels": 8294400,
                "maxAspectRatio": 3
            }
        })
    } else {
        json!({ "type": "enum", "default": "auto", "options": sizes })
    };
    let background = if is_gpt_image_2 {
        json!({ "type": "enum", "default": "auto", "options": ["auto", "opaque"] })
    } else {
        json!({ "type": "enum", "default": "auto", "options": ["auto", "transparent", "opaque"] })
    };
    let mut schema = json!({
        "meta": {
            "source": "official_catalog",
            "modelFamily": "gpt-image",
            "schemaVersion": "2026-07-21",
            "reference": "https://developers.openai.com/api/reference/resources/images/methods/generate"
        },
        "parameters": {
            "aspect_ratio": { "type": "enum", "default": "auto", "options": aspect_ratios },
            "size": size_definition,
            "quality": { "type": "enum", "default": "auto", "options": ["auto", "low", "medium", "high"] },
            "n": { "type": "integer", "default": 1, "min": 1, "max": 10 },
            "output_format": { "type": "enum", "default": "png", "options": ["png", "jpeg", "webp"] },
            "output_compression": {
                "type": "integer", "default": 100, "min": 0, "max": 100,
                "visible_when": { "output_format": ["jpeg", "webp"] }
            },
            "background": background,
            "moderation": { "type": "enum", "default": "auto", "options": ["auto", "low"] },
            "partial_images": {
                "type": "integer", "default": 0, "min": 0, "max": 3,
                "visible_when": { "stream": true }
            }
        }
    });
    if !is_gpt_image_2
        && let Some(parameters) = schema.get_mut("parameters").and_then(Value::as_object_mut)
    {
        parameters.insert(
            "input_fidelity".to_owned(),
            json!({
                "type": "enum",
                "default": "low",
                "options": ["low", "high"],
                "operations": ["edit"]
            }),
        );
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_image_model_gets_parameter_schema() {
        let entry = catalog_for("openai-compatible", "gpt-image-1");
        assert!(entry.verified);
        assert_eq!(entry.parameters["parameters"]["size"]["default"], "auto");
        assert_eq!(entry.capabilities["max_partial_images"], 3);
        assert_eq!(entry.parameters["parameters"]["partial_images"]["max"], 3);
        assert_eq!(
            entry.capabilities["image_edit_capability"]["supportsOutpaint"],
            true
        );
    }

    #[test]
    fn unknown_model_stays_unclassified() {
        assert!(!catalog_for("openai-compatible", "gpt-unknown").verified);
    }

    #[test]
    fn native_image_models_get_provider_specific_schema() {
        let gemini = catalog_for("gemini", "gemini-2.5-flash-image");
        assert!(gemini.verified);
        assert!(
            gemini.parameters["parameters"]["aspect_ratio"]["options"]
                .as_array()
                .is_some_and(|options| options.contains(&json!("21:9")))
        );
        assert_eq!(
            catalog_for("grok", "grok-imagine-image").parameters["parameters"]["resolution"]["default"],
            "1k"
        );
    }

    #[test]
    fn openai_model_families_keep_their_real_parameter_differences() {
        let image_2 = catalog_for("openai-compatible", "gpt-image-2");
        assert_eq!(
            image_2.capabilities["supports_transparent_background"],
            false
        );
        assert_eq!(
            image_2.parameters["parameters"]["background"]["options"],
            json!(["auto", "opaque"])
        );
        assert_eq!(
            image_2.parameters["parameters"]["size"]["allow_custom"],
            true
        );
        assert!(
            image_2.parameters["parameters"]
                .get("input_fidelity")
                .is_none()
        );

        let image_15 = catalog_for("openai-compatible", "gpt-image-1.5");
        assert_eq!(
            image_15.parameters["parameters"]["input_fidelity"]["operations"],
            json!(["edit"])
        );

        let dalle_2 = catalog_for("openai-compatible", "dall-e-2");
        assert_eq!(dalle_2.parameters["parameters"]["n"]["max"], 10);
        assert_eq!(
            dalle_2.parameters["parameters"]["size"]["options"],
            json!(["auto", "256x256", "512x512", "1024x1024"])
        );

        let dalle_3 = catalog_for("openai-compatible", "dall-e-3");
        assert_eq!(dalle_3.parameters["parameters"]["n"]["max"], 1);
        assert_eq!(
            dalle_3.parameters["parameters"]["style"]["options"],
            json!(["vivid", "natural"])
        );
    }

    #[test]
    fn pricing_validation_accepts_decimal_and_rejects_exponents() {
        assert_eq!(validate_price("0.125").unwrap(), "0.125");
        assert!(validate_price("1e3").is_err());
        assert!(validate_price("-1").is_err());
        assert_eq!(validate_currency("cny").unwrap(), "CNY");
    }
}
