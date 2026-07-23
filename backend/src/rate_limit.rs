use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{sync::Mutex, time::Instant};

use crate::{AppState, Settings, security::session_token};

#[derive(Clone)]
pub struct RateLimiter {
    enabled: bool,
    window: Duration,
    ip_limit: u32,
    session_limit: u32,
    user_limit: u32,
    entries: Arc<Mutex<HashMap<String, WindowEntry>>>,
}

struct WindowEntry {
    started_at: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(settings: &Settings) -> Self {
        Self {
            enabled: settings.rate_limit_enabled,
            window: Duration::from_secs(settings.rate_limit_window_seconds),
            ip_limit: settings.rate_limit_ip_requests,
            session_limit: settings.rate_limit_session_requests,
            user_limit: settings.rate_limit_user_requests,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn check(
        &self,
        ip: &str,
        token: Option<&str>,
        user_id: Option<uuid::Uuid>,
    ) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        let mut limits = Vec::with_capacity(3);
        limits.push((identity_key("ip", ip.as_bytes()), self.ip_limit));
        if let Some(token) = token {
            limits.push((
                identity_key("session", token.as_bytes()),
                self.session_limit,
            ));
        }
        if let Some(user_id) = user_id {
            limits.push((identity_key("user", user_id.as_bytes()), self.user_limit));
        }

        let now = Instant::now();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| now.duration_since(entry.started_at) < self.window);
        let mut retry_after = None;
        for (key, maximum) in &limits {
            if let Some(entry) = entries.get(key)
                && entry.count >= *maximum
            {
                retry_after = Some(
                    self.window
                        .saturating_sub(now.duration_since(entry.started_at))
                        .as_secs()
                        .max(1),
                );
                break;
            }
        }
        if retry_after.is_some() {
            return retry_after;
        }
        for (key, _) in limits {
            let entry = entries.entry(key).or_insert(WindowEntry {
                started_at: now,
                count: 0,
            });
            entry.count = entry.count.saturating_add(1);
        }
        None
    }
}

pub async fn enforce(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if !path.starts_with("/api/") || matches!(path, "/api/v1/health" | "/api/v1/ready") {
        return next.run(request).await;
    }
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let token = session_token(request.headers());
    let user_id = token
        .as_deref()
        .and_then(|value| state.sessions.decode(value).ok())
        .map(|claims| claims.sub);
    if let Some(retry_after) = state
        .rate_limiter
        .check(&ip, token.as_deref(), user_id)
        .await
    {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": {
                    "code": "RATE_LIMITED",
                    "message": "too many requests; retry later"
                }
            })),
        )
            .into_response();
        if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }
    next.run(request).await
}

fn identity_key(scope: &str, raw: &[u8]) -> String {
    format!("{scope}:{}", hex::encode(Sha256::digest(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(maximum: u32) -> RateLimiter {
        RateLimiter {
            enabled: true,
            window: Duration::from_secs(60),
            ip_limit: maximum,
            session_limit: maximum,
            user_limit: maximum,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tokio::test]
    async fn limits_each_hashed_identity_without_storing_raw_values() {
        let limiter = limiter(1);
        let user_id = uuid::Uuid::new_v4();
        assert!(
            limiter
                .check("203.0.113.9", Some("raw-session-token"), Some(user_id))
                .await
                .is_none()
        );
        assert!(
            limiter
                .check("203.0.113.9", Some("raw-session-token"), Some(user_id))
                .await
                .is_some()
        );
        let keys: Vec<_> = limiter.entries.lock().await.keys().cloned().collect();
        assert!(keys.iter().all(|key| !key.contains("203.0.113.9")));
        assert!(keys.iter().all(|key| !key.contains("raw-session-token")));
    }
}
