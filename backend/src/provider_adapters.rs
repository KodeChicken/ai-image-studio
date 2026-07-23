use std::fmt;

use anyhow::{Context, bail};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::{
    Client, Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE},
};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{DeserializeOwned, Error as _};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

const MODEL_LIST_LIMIT: usize = 8 * 1024 * 1024;
const UPSTREAM_ERROR_LIMIT: usize = 16 * 1024;

#[derive(Debug)]
struct ProviderHttpError {
    operation: String,
    status: StatusCode,
    code: Option<String>,
    error_type: Option<String>,
    message: Option<String>,
    request_id: Option<String>,
    moderation_stage: Option<String>,
    moderation_categories: Vec<String>,
}

impl fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "provider {} returned HTTP {}",
            self.operation, self.status
        )?;
        let mut metadata = Vec::new();
        if let Some(code) = &self.code {
            metadata.push(format!("code={code}"));
        }
        if let Some(error_type) = &self.error_type {
            metadata.push(format!("type={error_type}"));
        }
        if let Some(stage) = &self.moderation_stage {
            metadata.push(format!("moderation_stage={stage}"));
        }
        if !self.moderation_categories.is_empty() {
            metadata.push(format!(
                "moderation_categories={}",
                self.moderation_categories.join(",")
            ));
        }
        if let Some(request_id) = &self.request_id {
            metadata.push(format!("request_id={request_id}"));
        }
        if !metadata.is_empty() {
            write!(formatter, " [{}]", metadata.join(", "))?;
        }
        if let Some(message) = &self.message {
            write!(formatter, ": {message}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProviderHttpError {}

pub(crate) struct ProviderInput {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Bytes,
}

pub(crate) struct ProviderRequest {
    pub model: String,
    pub prompt: String,
    pub parameters: Value,
    pub inputs: Vec<ProviderInput>,
    pub max_image_bytes: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderImage {
    pub url: Option<String>,
    pub b64_json: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ProviderPartialImage {
    pub index: u32,
    pub b64_json: String,
}

type PartialImageSender = mpsc::UnboundedSender<ProviderPartialImage>;

pub(crate) struct DiscoveredModel {
    pub id: String,
    pub display_name: Option<String>,
    pub metadata: Map<String, Value>,
}

#[async_trait]
trait ImageProviderAdapter: Sync {
    async fn list_models(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
    ) -> anyhow::Result<Vec<DiscoveredModel>>;

    async fn generate(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>>;

    async fn edit(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>>;
}

struct OpenAiCompatibleAdapter;
struct GeminiAdapter;
struct GrokAdapter;

static OPENAI_COMPATIBLE: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter;
static GEMINI: GeminiAdapter = GeminiAdapter;
static GROK: GrokAdapter = GrokAdapter;

fn adapter(provider_type: &str) -> anyhow::Result<&'static dyn ImageProviderAdapter> {
    match provider_type {
        "openai-compatible" => Ok(&OPENAI_COMPATIBLE),
        "gemini" => Ok(&GEMINI),
        "grok" => Ok(&GROK),
        _ => bail!("provider type '{provider_type}' is not implemented for image generation"),
    }
}

pub(crate) async fn list_models(
    provider_type: &str,
    client: &Client,
    base_url: &str,
    credential: &SecretString,
) -> anyhow::Result<Vec<DiscoveredModel>> {
    adapter(provider_type)?
        .list_models(client, base_url, credential)
        .await
}

pub(crate) async fn create_images(
    provider_type: &str,
    operation: &str,
    client: &Client,
    base_url: &str,
    credential: &SecretString,
    request: ProviderRequest,
) -> anyhow::Result<Vec<ProviderImage>> {
    create_images_with_partials(
        provider_type,
        operation,
        client,
        base_url,
        credential,
        request,
        None,
    )
    .await
}

pub(crate) async fn create_images_with_partials(
    provider_type: &str,
    operation: &str,
    client: &Client,
    base_url: &str,
    credential: &SecretString,
    request: ProviderRequest,
    partial_sender: Option<PartialImageSender>,
) -> anyhow::Result<Vec<ProviderImage>> {
    let adapter = adapter(provider_type)?;
    match operation {
        "generation" => {
            adapter
                .generate(client, base_url, credential, request, partial_sender)
                .await
        }
        "edit" => {
            adapter
                .edit(client, base_url, credential, request, partial_sender)
                .await
        }
        _ => bail!("unsupported image operation '{operation}'"),
    }
}

#[async_trait]
impl ImageProviderAdapter for OpenAiCompatibleAdapter {
    async fn list_models(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
    ) -> anyhow::Result<Vec<DiscoveredModel>> {
        list_openai_models(
            client,
            &format!("{}/models", base_url.trim_end_matches('/')),
            credential,
        )
        .await
    }

    async fn generate(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_openai_images(
            client,
            base_url,
            credential,
            request,
            false,
            false,
            partial_sender,
        )
        .await
    }

    async fn edit(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_openai_images(
            client,
            base_url,
            credential,
            request,
            true,
            false,
            partial_sender,
        )
        .await
    }
}

#[async_trait]
impl ImageProviderAdapter for GrokAdapter {
    async fn list_models(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
    ) -> anyhow::Result<Vec<DiscoveredModel>> {
        list_openai_models(
            client,
            &format!("{}/models", versioned_base_url(base_url)),
            credential,
        )
        .await
    }

    async fn generate(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        _partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_openai_images(
            client,
            &versioned_base_url(base_url),
            credential,
            request,
            false,
            true,
            None,
        )
        .await
    }

    async fn edit(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        _partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_openai_images(
            client,
            &versioned_base_url(base_url),
            credential,
            request,
            true,
            true,
            None,
        )
        .await
    }
}

#[async_trait]
impl ImageProviderAdapter for GeminiAdapter {
    async fn list_models(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
    ) -> anyhow::Result<Vec<DiscoveredModel>> {
        let endpoint = format!("{}/models?pageSize=1000", gemini_base_url(base_url));
        let response = client
            .get(endpoint)
            .header("x-goog-api-key", credential.expose_secret())
            .send()
            .await?;
        let body: GeminiModelsResponse =
            read_json(response, MODEL_LIST_LIMIT, "model list").await?;
        let mut discovered = Vec::new();
        for value in body.models {
            let Some(object) = value.as_object() else {
                continue;
            };
            let Some(name) = object.get("name").and_then(Value::as_str) else {
                continue;
            };
            let id = name.strip_prefix("models/").unwrap_or(name).trim();
            if id.is_empty() {
                continue;
            }
            discovered.push(DiscoveredModel {
                id: id.to_owned(),
                display_name: object
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                metadata: object.clone(),
            });
        }
        Ok(discovered)
    }

    async fn generate(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        _partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_gemini(client, base_url, credential, request).await
    }

    async fn edit(
        &self,
        client: &Client,
        base_url: &str,
        credential: &SecretString,
        request: ProviderRequest,
        _partial_sender: Option<PartialImageSender>,
    ) -> anyhow::Result<Vec<ProviderImage>> {
        call_gemini(client, base_url, credential, request).await
    }
}

async fn list_openai_models(
    client: &Client,
    endpoint: &str,
    credential: &SecretString,
) -> anyhow::Result<Vec<DiscoveredModel>> {
    let response = client
        .get(endpoint)
        .bearer_auth(credential.expose_secret())
        .send()
        .await?;
    let body: OpenAiModelsResponse = read_json(response, MODEL_LIST_LIMIT, "model list").await?;
    Ok(body
        .data
        .into_iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let id = object.get("id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            Some(DiscoveredModel {
                id: id.to_owned(),
                display_name: object
                    .get("display_name")
                    .or_else(|| object.get("displayName"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                metadata: object.clone(),
            })
        })
        .collect())
}

async fn call_openai_images(
    client: &Client,
    base_url: &str,
    credential: &SecretString,
    request: ProviderRequest,
    edit: bool,
    grok: bool,
    partial_sender: Option<PartialImageSender>,
) -> anyhow::Result<Vec<ProviderImage>> {
    let response_limit = image_response_limit(&request);
    let parameters = openai_parameters(&request.parameters, grok, &request.model, edit);
    let native_stream = parameters
        .iter()
        .any(|(key, value)| key == "stream" && value == &json!(true));
    let response = if edit {
        let endpoint = format!("{}/images/edits", base_url.trim_end_matches('/'));
        let mut form = reqwest::multipart::Form::new()
            .text("model", request.model)
            .text("prompt", request.prompt);
        for input in request.inputs {
            let part = reqwest::multipart::Part::bytes(input.bytes.to_vec())
                .file_name(input.filename)
                .mime_str(&input.mime_type)?;
            form = form.part("image[]", part);
        }
        for (key, value) in parameters {
            form = form.text(key, scalar_text(&value)?);
        }
        let request = client
            .post(endpoint)
            .bearer_auth(credential.expose_secret())
            .multipart(form);
        let request = if native_stream {
            request.header(ACCEPT, "text/event-stream")
        } else {
            request
        };
        request.send().await?
    } else {
        let endpoint = format!("{}/images/generations", base_url.trim_end_matches('/'));
        let mut body = Map::new();
        body.insert("model".to_owned(), json!(request.model));
        body.insert("prompt".to_owned(), json!(request.prompt));
        for (key, value) in parameters {
            body.insert(key, value);
        }
        let request = client
            .post(endpoint)
            .bearer_auth(credential.expose_secret())
            .json(&body);
        let request = if native_stream {
            request.header(ACCEPT, "text/event-stream")
        } else {
            request
        };
        request.send().await?
    };
    let provider_returned_json = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if native_stream && !provider_returned_json {
        read_openai_image_stream(response, response_limit, partial_sender.as_ref()).await
    } else {
        let body: OpenAiImagesResponse =
            read_json(response, response_limit, "image generation").await?;
        Ok(body.data)
    }
}

async fn call_gemini(
    client: &Client,
    base_url: &str,
    credential: &SecretString,
    request: ProviderRequest,
) -> anyhow::Result<Vec<ProviderImage>> {
    validate_model_id(&request.model)?;
    let response_limit = image_response_limit(&request);
    let mut parts = Vec::with_capacity(request.inputs.len() + 1);
    for input in request.inputs {
        parts.push(json!({
            "inlineData": {
                "mimeType": input.mime_type,
                "data": STANDARD.encode(input.bytes)
            }
        }));
    }
    parts.push(json!({ "text": request.prompt }));
    let mut generation_config = Map::new();
    generation_config.insert("responseModalities".to_owned(), json!(["TEXT", "IMAGE"]));
    let mut image_config = Map::new();
    if let Some(value) = request
        .parameters
        .get("aspect_ratio")
        .and_then(Value::as_str)
        .filter(|value| *value != "auto")
    {
        image_config.insert("aspectRatio".to_owned(), json!(value));
    }
    if let Some(value) = request
        .parameters
        .get("size")
        .and_then(Value::as_str)
        .filter(|value| *value != "auto")
    {
        image_config.insert("imageSize".to_owned(), json!(value.to_ascii_uppercase()));
    }
    if !image_config.is_empty() {
        generation_config.insert("imageConfig".to_owned(), Value::Object(image_config));
    }
    let endpoint = format!(
        "{}/models/{}:generateContent",
        gemini_base_url(base_url),
        request.model
    );
    let response = client
        .post(endpoint)
        .header("x-goog-api-key", credential.expose_secret())
        .json(&json!({
            "contents": [{ "role": "user", "parts": parts }],
            "generationConfig": generation_config
        }))
        .send()
        .await?;
    let body: GeminiGenerateResponse =
        read_json(response, response_limit, "image generation").await?;
    let mut images = Vec::new();
    let mut finish_reasons = Vec::new();
    for candidate in body.candidates {
        if let Some(reason) = candidate.finish_reason {
            finish_reasons.push(reason);
        }
        let Some(content) = candidate.content else {
            continue;
        };
        for part in content.parts {
            if let Some(inline) = part.inline_data {
                images.push(ProviderImage {
                    url: None,
                    b64_json: Some(inline.data),
                });
            } else if let Some(file) = part.file_data {
                images.push(ProviderImage {
                    url: Some(file.file_uri),
                    b64_json: None,
                });
            }
        }
    }
    if images.is_empty() && !finish_reasons.is_empty() {
        bail!(
            "Gemini returned no image (finish reason: {})",
            finish_reasons.join(", ")
        );
    }
    Ok(images)
}

fn versioned_base_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        base.to_owned()
    } else {
        format!("{base}/v1")
    }
}

fn gemini_base_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1beta") {
        base.to_owned()
    } else {
        format!("{base}/v1beta")
    }
}

fn validate_model_id(value: &str) -> anyhow::Result<()> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        bail!("provider returned an invalid model ID")
    }
}

pub(crate) fn openai_parameters(
    params: &Value,
    grok: bool,
    model: &str,
    edit: bool,
) -> Vec<(String, Value)> {
    const OPENAI_ALLOWED: &[&str] = &[
        "size",
        "quality",
        "n",
        "output_format",
        "output_compression",
        "background",
        "moderation",
        "response_format",
        "style",
        "input_fidelity",
        "partial_images",
    ];
    const GROK_ALLOWED: &[&str] = &["n", "aspect_ratio", "resolution", "response_format"];
    let allowed = if grok { GROK_ALLOWED } else { OPENAI_ALLOWED };
    let mut values: Vec<_> = params
        .as_object()
        .into_iter()
        .flat_map(|object| object.iter())
        .filter(|(key, value)| allowed.contains(&key.as_str()) && !value.is_null())
        .filter(|(_, value)| value.as_str() != Some("auto"))
        .filter(|(key, _)| key.as_str() != "input_fidelity" || edit)
        .filter(|(key, value)| key.as_str() != "partial_images" || value.as_u64().unwrap_or(0) > 0)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !grok {
        values.push(("stream".to_owned(), json!(true)));
    }
    if !grok
        && !values.iter().any(|(key, _)| key == "size")
        && let Some(aspect_ratio) = params.get("aspect_ratio").and_then(Value::as_str)
        && let Some(size) = size_for_aspect_ratio(model, aspect_ratio)
    {
        values.push(("size".to_owned(), json!(size)));
    }
    values
}

fn size_for_aspect_ratio(model: &str, aspect_ratio: &str) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    if model == "dall-e-3" {
        return match aspect_ratio {
            "1:1" => Some("1024x1024"),
            "16:9" => Some("1792x1024"),
            "9:16" => Some("1024x1792"),
            _ => None,
        };
    }
    if model == "dall-e-2" {
        return (aspect_ratio == "1:1").then_some("1024x1024");
    }
    if model == "gpt-image-2" || model.starts_with("gpt-image-2-") {
        return match aspect_ratio {
            "1:1" => Some("1024x1024"),
            "3:2" => Some("1536x1024"),
            "2:3" => Some("1024x1536"),
            "16:9" => Some("1536x864"),
            "9:16" => Some("864x1536"),
            _ => None,
        };
    }
    match aspect_ratio {
        "1:1" => Some("1024x1024"),
        "3:2" => Some("1536x1024"),
        "2:3" => Some("1024x1536"),
        _ => None,
    }
}

fn scalar_text(value: &Value) -> anyhow::Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => bail!("provider parameter must be scalar"),
    }
}

fn image_response_limit(request: &ProviderRequest) -> usize {
    let count = request
        .parameters
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 10) as usize;
    let partial_count = request
        .parameters
        .get("partial_images")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .clamp(0, 3) as usize;
    request
        .max_image_bytes
        .saturating_mul(count)
        .saturating_mul(partial_count.saturating_add(1))
        .saturating_mul(4)
        .saturating_div(3)
        .saturating_add(1024 * 1024)
}

async fn read_openai_image_stream(
    response: Response,
    limit: usize,
    partial_sender: Option<&PartialImageSender>,
) -> anyhow::Result<Vec<ProviderImage>> {
    let status = response.status();
    if !status.is_success() {
        return Err(provider_http_error(response, "image generation").await);
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("provider image generation response is too large");
    }
    let mut received = 0usize;
    let mut pending = Vec::new();
    let mut state = OpenAiImageStreamState::default();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        received = received.saturating_add(chunk.len());
        if received > limit {
            bail!("provider image generation response is too large");
        }
        pending.extend_from_slice(&chunk);
        while let Some((index, boundary_length)) = sse_event_boundary(&pending) {
            let event = pending.drain(..index).collect::<Vec<_>>();
            pending.drain(..boundary_length);
            let event =
                std::str::from_utf8(&event).context("invalid provider image stream encoding")?;
            state.accept_event(event, partial_sender)?;
        }
    }
    if pending.iter().any(|byte| !byte.is_ascii_whitespace()) {
        let event =
            std::str::from_utf8(&pending).context("invalid provider image stream encoding")?;
        state.accept_event(event, partial_sender)?;
    }
    state.finish()
}

#[cfg(test)]
fn parse_openai_image_stream(body: &str) -> anyhow::Result<Vec<ProviderImage>> {
    let normalized = body.replace("\r\n", "\n");
    let mut state = OpenAiImageStreamState::default();
    for event in normalized.split("\n\n") {
        state.accept_event(event, None)?;
    }
    state.finish()
}

fn sse_event_boundary(body: &[u8]) -> Option<(usize, usize)> {
    let lf = body.windows(2).position(|window| window == b"\n\n");
    let crlf = body.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(lf), Some(crlf)) if lf <= crlf => Some((lf, 2)),
        (Some(_), Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
    }
}

#[derive(Default)]
struct OpenAiImageStreamState {
    final_response: Option<Vec<ProviderImage>>,
    final_image: Option<ProviderImage>,
}

impl OpenAiImageStreamState {
    fn accept_event(
        &mut self,
        event: &str,
        partial_sender: Option<&PartialImageSender>,
    ) -> anyhow::Result<()> {
        let payload = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if payload.is_empty() || payload == "[DONE]" {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&payload)
            .context("invalid provider image generation stream event")?;
        if let Some(error) = value.get("error") {
            bail!("provider image generation stream failed: {error}");
        }
        if value.get("data").is_some() {
            self.final_response = Some(
                serde_json::from_value::<OpenAiImagesResponse>(value)
                    .context("invalid provider image generation stream result")?
                    .data,
            );
            return Ok(());
        }
        if let Some(b64_json) = value.get("b64_json").and_then(Value::as_str) {
            let event_type = value.get("type").and_then(Value::as_str);
            if event_type.is_some_and(|event_type| event_type.ends_with(".partial_image")) {
                if let Some(sender) = partial_sender {
                    let index = value
                        .get("partial_image_index")
                        .and_then(Value::as_u64)
                        .unwrap_or_default()
                        .min(u32::MAX as u64) as u32;
                    let _ = sender.send(ProviderPartialImage {
                        index,
                        b64_json: b64_json.to_owned(),
                    });
                }
                return Ok(());
            }
            if event_type.is_some_and(|event_type| event_type.ends_with(".completed")) {
                self.final_image = Some(ProviderImage {
                    url: None,
                    b64_json: Some(b64_json.to_owned()),
                });
            }
        }
        Ok(())
    }

    fn finish(self) -> anyhow::Result<Vec<ProviderImage>> {
        if let Some(images) = self.final_response.filter(|images| !images.is_empty()) {
            return Ok(images);
        }
        Ok(vec![self.final_image.context(
            "provider image stream ended before a completed image was received",
        )?])
    }
}

async fn read_json<T: DeserializeOwned>(
    response: Response,
    limit: usize,
    operation: &str,
) -> anyhow::Result<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(provider_http_error(response, operation).await);
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("provider {operation} response is too large");
    }
    let body = response.bytes().await?;
    if body.len() > limit {
        bail!("provider {operation} response is too large");
    }
    serde_json::from_slice(&body).with_context(|| format!("invalid provider {operation} response"))
}

async fn provider_http_error(mut response: Response, operation: &str) -> anyhow::Error {
    let status = response.status();
    let header_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| normalized_error_text(value, 256));
    let mut body = Vec::new();
    while body.len() < UPSTREAM_ERROR_LIMIT {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = UPSTREAM_ERROR_LIMIT - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            Ok(None) | Err(_) => break,
        }
    }
    anyhow::Error::new(parse_provider_http_error(
        status,
        operation,
        header_request_id,
        &body,
    ))
}

fn parse_provider_http_error(
    status: StatusCode,
    operation: &str,
    header_request_id: Option<String>,
    body: &[u8],
) -> ProviderHttpError {
    let json = serde_json::from_slice::<Value>(body).ok();
    let error = json
        .as_ref()
        .and_then(|value| value.get("error").or(Some(value)));
    let message = error
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("detail"))
                .or_else(|| value.is_string().then_some(value))
        })
        .and_then(Value::as_str)
        .and_then(|value| normalized_error_text(value, 500))
        .or_else(|| {
            json.is_none()
                .then(|| String::from_utf8(body.to_vec()).ok())
                .flatten()
                .and_then(|value| normalized_error_text(&value, 500))
        });
    let code = error
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .and_then(|value| normalized_error_text(value, 128));
    let error_type = error
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .and_then(|value| normalized_error_text(value, 128));
    let request_id = header_request_id.or_else(|| {
        error
            .and_then(|value| value.get("request_id"))
            .or_else(|| json.as_ref().and_then(|value| value.get("request_id")))
            .and_then(Value::as_str)
            .and_then(|value| normalized_error_text(value, 256))
    });
    let moderation_stage = error
        .and_then(|value| value.pointer("/moderation_details/moderation_stage"))
        .and_then(Value::as_str)
        .and_then(|value| normalized_error_text(value, 32));
    let moderation_categories = error
        .and_then(|value| value.pointer("/moderation_details/categories"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|value| normalized_error_text(value, 64))
        .take(16)
        .collect();
    ProviderHttpError {
        operation: operation.to_owned(),
        status,
        code,
        error_type,
        message,
        request_id,
        moderation_stage,
        moderation_categories,
    }
}

fn normalized_error_text(value: &str, limit: usize) -> Option<String> {
    let normalized = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn structured_provider_error(error: &anyhow::Error) -> Option<&ProviderHttpError> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ProviderHttpError>())
}

pub(crate) fn provider_error_status(error: &anyhow::Error) -> Option<u16> {
    structured_provider_error(error).map(|error| error.status.as_u16())
}

pub(crate) fn provider_error_code(error: &anyhow::Error) -> Option<&str> {
    structured_provider_error(error).and_then(|error| error.code.as_deref())
}

pub(crate) fn provider_error_is_retryable(error: &anyhow::Error) -> bool {
    let Some(error) = structured_provider_error(error) else {
        return true;
    };
    if error.error_type.as_deref() == Some("image_generation_user_error") {
        return false;
    }
    error.status == StatusCode::TOO_MANY_REQUESTS || error.status.is_server_error()
}

pub(crate) fn provider_error_user_message(error: &anyhow::Error) -> Option<String> {
    let error = structured_provider_error(error)?;
    if error.code.as_deref() == Some("moderation_blocked")
        || error.message.as_deref().is_some_and(|message| {
            let message = message.to_ascii_lowercase();
            message.contains("moderation")
                || message.contains("safety system")
                || message.contains("content policy")
        })
    {
        return Some(match error.moderation_stage.as_deref() {
            Some("input") => {
                "图片生成请求未通过安全检查，请调整提示词或输入图片后重试。".to_owned()
            }
            Some("output") => "生成结果未通过安全检查，请调整提示词后重新生成。".to_owned(),
            _ => "本次图片生成未通过安全检查，请调整描述后重试。".to_owned(),
        });
    }
    if error.error_type.as_deref() == Some("image_generation_user_error")
        || error.status == StatusCode::BAD_REQUEST
    {
        return Some(match &error.message {
            Some(message) => format!("模型拒绝了本次图片生成请求：{message}"),
            None => "模型拒绝了本次图片生成请求，请调整提示词或生成参数后重试。".to_owned(),
        });
    }
    Some(match error.status {
        StatusCode::UNAUTHORIZED => "Provider 认证失败，请检查 API Key。".to_owned(),
        StatusCode::FORBIDDEN => "Provider 拒绝访问，请检查账号或模型权限。".to_owned(),
        StatusCode::TOO_MANY_REQUESTS => "上游生图服务请求过多，请稍后重试。".to_owned(),
        status if status.is_server_error() => "上游生图服务暂时不可用，请稍后重试。".to_owned(),
        _ => error
            .message
            .as_ref()
            .map(|message| format!("上游生图请求失败：{message}"))
            .unwrap_or_else(|| "上游生图请求失败，请检查 Provider 配置。".to_owned()),
    })
}

#[derive(serde::Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(serde::Deserialize)]
struct OpenAiImagesResponse {
    #[serde(default)]
    data: Vec<ProviderImage>,
}

impl<'de> serde::Deserialize<'de> for ProviderImage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let image = Self {
            url: value.get("url").and_then(Value::as_str).map(str::to_owned),
            b64_json: value
                .get("b64_json")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        if image.url.is_none() && image.b64_json.is_none() {
            return Err(D::Error::custom(
                "provider image has neither URL nor base64 data",
            ));
        }
        Ok(image)
    }
}

#[derive(serde::Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<Value>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerateResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: Option<GeminiContent>,
    finish_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    inline_data: Option<GeminiInlineData>,
    file_data: Option<GeminiFileData>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    #[allow(dead_code)]
    mime_type: String,
    data: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFileData {
    file_uri: String,
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        response::Response,
        routing::any,
    };
    use tokio::net::TcpListener;

    use super::*;

    async fn mock_server(
        handler: impl Fn(Request<Body>) -> Response<Body> + Clone + Send + Sync + 'static,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        let app = Router::new().fallback(any(move |request| {
            let handler = handler.clone();
            async move { handler(request) }
        }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn gemini_contract_discovers_models_and_parses_inline_images() {
        let base_url = mock_server(|request| {
            assert_eq!(request.headers()["x-goog-api-key"], "gemini-secret");
            let path = request.uri().path();
            if path.ends_with("/models") {
                return Response::new(Body::from(
                    json!({ "models": [{ "name": "models/gemini-2.5-flash-image", "displayName": "Gemini Flash Image" }] }).to_string(),
                ));
            }
            assert_eq!(
                path,
                "/v1beta/models/gemini-2.5-flash-image:generateContent"
            );
            Response::new(Body::from(
                json!({ "candidates": [{ "finishReason": "STOP", "content": { "parts": [{ "inlineData": { "mimeType": "image/png", "data": "aW1hZ2U=" } }] } }] }).to_string(),
            ))
        })
        .await;
        let credential = SecretString::from("gemini-secret".to_owned());
        let models = list_models("gemini", &Client::new(), &base_url, &credential)
            .await
            .unwrap();
        assert_eq!(models[0].id, "gemini-2.5-flash-image");
        let images = create_images(
            "gemini",
            "generation",
            &Client::new(),
            &base_url,
            &credential,
            ProviderRequest {
                model: models[0].id.clone(),
                prompt: "draw a cat".to_owned(),
                parameters: json!({ "aspect_ratio": "16:9", "size": "2k" }),
                inputs: Vec::new(),
                max_image_bytes: 1024,
            },
        )
        .await
        .unwrap();
        assert_eq!(images[0].b64_json.as_deref(), Some("aW1hZ2U="));
    }

    #[tokio::test]
    async fn grok_contract_uses_versioned_images_api() {
        let base_url = mock_server(|request| {
            assert_eq!(request.headers()["authorization"], "Bearer grok-secret");
            assert_eq!(request.uri().path(), "/v1/images/generations");
            Response::new(Body::from(
                json!({ "data": [{ "b64_json": "aW1hZ2U=" }] }).to_string(),
            ))
        })
        .await;
        let credential = SecretString::from("grok-secret".to_owned());
        let images = create_images(
            "grok",
            "generation",
            &Client::new(),
            &base_url,
            &credential,
            ProviderRequest {
                model: "grok-imagine-image".to_owned(),
                prompt: "draw a cat".to_owned(),
                parameters: json!({ "aspect_ratio": "16:9", "resolution": "2k" }),
                inputs: Vec::new(),
                max_image_bytes: 1024,
            },
        )
        .await
        .unwrap();
        assert_eq!(images.len(), 1);
    }

    #[tokio::test]
    async fn openai_contract_rejects_http_errors_and_invalid_image_payloads() {
        let credential = SecretString::from("secret".to_owned());
        for (response, expected) in [
            (
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from(r#"{"error":{"message":"bad request"}}"#))
                    .unwrap(),
                "HTTP 400 Bad Request: bad request",
            ),
            (
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from(r#"{"error":{"message":"unavailable"}}"#))
                    .unwrap(),
                "HTTP 503 Service Unavailable: unavailable",
            ),
            (
                Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from("not-json"))
                    .unwrap(),
                "invalid provider image generation response",
            ),
            (
                Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"data":[{"revised_prompt":"cat"}]}"#))
                    .unwrap(),
                "invalid provider image generation response",
            ),
        ] {
            let response = Arc::new(std::sync::Mutex::new(Some(response)));
            let base_url = mock_server(move |_| {
                response
                    .lock()
                    .unwrap()
                    .take()
                    .expect("mock response is used once")
            })
            .await;
            let error = create_images(
                "openai-compatible",
                "generation",
                &Client::new(),
                &base_url,
                &credential,
                ProviderRequest {
                    model: "gpt-image-1".to_owned(),
                    prompt: "draw a cat".to_owned(),
                    parameters: json!({ "size": "auto", "n": 1 }),
                    inputs: Vec::new(),
                    max_image_bytes: 1024,
                },
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected '{expected}' in '{error}'"
            );
        }
    }

    #[test]
    fn openai_image_errors_preserve_rejection_details_and_retry_only_transient_failures() {
        let blocked = anyhow::Error::new(parse_provider_http_error(
            StatusCode::BAD_REQUEST,
            "image generation",
            Some("req_test_123".to_owned()),
            br#"{
                "error": {
                    "message": "The request was rejected by the safety system.",
                    "type": "image_generation_user_error",
                    "code": "moderation_blocked",
                    "moderation_details": {
                        "moderation_stage": "input",
                        "categories": ["sexual"]
                    }
                }
            }"#,
        ));
        assert_eq!(provider_error_status(&blocked), Some(400));
        assert_eq!(provider_error_code(&blocked), Some("moderation_blocked"));
        assert!(!provider_error_is_retryable(&blocked));
        assert_eq!(
            provider_error_user_message(&blocked).as_deref(),
            Some("图片生成请求未通过安全检查，请调整提示词或输入图片后重试。")
        );
        let summary = blocked.to_string();
        assert!(summary.contains("code=moderation_blocked"));
        assert!(summary.contains("type=image_generation_user_error"));
        assert!(summary.contains("moderation_stage=input"));
        assert!(summary.contains("moderation_categories=sexual"));
        assert!(summary.contains("request_id=req_test_123"));

        for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY] {
            let transient = anyhow::Error::new(parse_provider_http_error(
                status,
                "image generation",
                None,
                br#"{"error":{"message":"temporary failure"}}"#,
            ));
            assert!(provider_error_is_retryable(&transient));
        }
    }

    #[tokio::test]
    async fn openai_contract_honors_the_http_client_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(any(|| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Response::new(Body::from(
                json!({ "data": [{ "b64_json": "aW1hZ2U=" }] }).to_string(),
            ))
        }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .unwrap();
        let error = create_images(
            "openai-compatible",
            "generation",
            &client,
            &format!("http://{address}"),
            &SecretString::from("secret".to_owned()),
            ProviderRequest {
                model: "gpt-image-1".to_owned(),
                prompt: "draw a cat".to_owned(),
                parameters: json!({ "size": "auto", "n": 1 }),
                inputs: Vec::new(),
                max_image_bytes: 1024,
            },
        )
        .await
        .unwrap_err();
        assert!(error.chain().any(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout)
        }));
    }

    #[tokio::test]
    async fn openai_contract_uses_native_image_streams_by_default() {
        let base_url = mock_server(|request| {
            assert_eq!(request.uri().path(), "/images/generations");
            assert_eq!(request.headers()["accept"], "text/event-stream");
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from(concat!(
                    ": keepalive\n\n",
                    "event: image_generation.completed\n",
                    "data: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZmluYWw=\"}\n\n"
                )))
                .unwrap()
        })
        .await;
        let images = create_images_with_partials(
            "openai-compatible",
            "generation",
            &Client::new(),
            &base_url,
            &SecretString::from("secret".to_owned()),
            ProviderRequest {
                model: "gpt-image-2".to_owned(),
                prompt: "draw a cat".to_owned(),
                parameters: json!({ "partial_images": 0 }),
                inputs: Vec::new(),
                max_image_bytes: 1024,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].b64_json.as_deref(), Some("ZmluYWw="));
    }

    #[tokio::test]
    async fn openai_contract_accepts_json_when_provider_ignores_streaming() {
        let base_url = mock_server(|request| {
            assert_eq!(request.headers()["accept"], "text/event-stream");
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "data": [{ "b64_json": "aW1hZ2U=" }] }).to_string(),
                ))
                .unwrap()
        })
        .await;
        let images = create_images(
            "openai-compatible",
            "generation",
            &Client::new(),
            &base_url,
            &SecretString::from("secret".to_owned()),
            ProviderRequest {
                model: "gpt-image-2".to_owned(),
                prompt: "draw a cat".to_owned(),
                parameters: json!({}),
                inputs: Vec::new(),
                max_image_bytes: 1024,
            },
        )
        .await
        .unwrap();
        assert_eq!(images[0].b64_json.as_deref(), Some("aW1hZ2U="));
    }

    #[test]
    fn openai_auto_size_uses_model_specific_aspect_ratio_mapping() {
        let gpt_image_2 = openai_parameters(
            &json!({ "size": "auto", "aspect_ratio": "16:9" }),
            false,
            "gpt-image-2",
            false,
        );
        assert!(gpt_image_2.contains(&("size".to_owned(), json!("1536x864"))));

        let dalle_3 = openai_parameters(
            &json!({ "size": "auto", "aspect_ratio": "16:9" }),
            false,
            "dall-e-3",
            false,
        );
        assert!(dalle_3.contains(&("size".to_owned(), json!("1792x1024"))));
    }

    #[test]
    fn openai_native_stream_is_enabled_without_partial_images() {
        let streamed =
            openai_parameters(&json!({ "partial_images": 2 }), false, "gpt-image-2", false);
        assert!(streamed.contains(&("partial_images".to_owned(), json!(2))));
        assert!(streamed.contains(&("stream".to_owned(), json!(true))));

        let without_partials =
            openai_parameters(&json!({ "partial_images": 0 }), false, "gpt-image-2", false);
        assert!(
            !without_partials
                .iter()
                .any(|(key, _)| key == "partial_images")
        );
        assert!(without_partials.contains(&("stream".to_owned(), json!(true))));

        let grok = openai_parameters(&json!({}), true, "grok-2-image", false);
        assert!(!grok.iter().any(|(key, _)| key == "stream"));
    }

    #[test]
    fn parses_openai_image_stream_and_returns_only_the_final_frame() {
        let images = parse_openai_image_stream(concat!(
            "event: image_generation.partial_image\r\n",
            "data: {\"type\":\"image_generation.partial_image\",\"partial_image_index\":0,\"b64_json\":\"cGFydGlhbA==\"}\r\n\r\n",
            "event: image_generation.completed\r\n",
            "data: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZmluYWw=\"}\r\n\r\n"
        ))
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].b64_json.as_deref(), Some("ZmluYWw="));
    }

    #[test]
    fn rejects_openai_image_stream_without_completed_event() {
        let error = parse_openai_image_stream(concat!(
            "event: image_generation.partial_image\n",
            "data: {\"type\":\"image_generation.partial_image\",\"partial_image_index\":0,\"b64_json\":\"cGFydGlhbA==\"}\n\n"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("ended before a completed image"));
    }
}
