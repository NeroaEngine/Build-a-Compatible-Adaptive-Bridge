#![cfg(feature = "headless-http")]

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
pub struct HeadlessCaptureRequest {
    pub request_id: Option<Uuid>,
    pub url: String,
    pub profile_id: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_scale")]
    pub device_scale_factor: f64,
    #[serde(default)]
    pub full_page: bool,
    #[serde(default)]
    pub geo_context: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct HeadlessCapture {
    pub capture_id: String,
    pub renderer: String,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub captured_at: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

#[async_trait]
pub trait HeadlessCaptureBackend: Send + Sync + 'static {
    async fn ready(&self) -> Result<(), String>;
    async fn capture(&self, request: HeadlessCaptureRequest) -> Result<HeadlessCapture, String>;
}

#[derive(Clone)]
struct AppState {
    backend: Arc<dyn HeadlessCaptureBackend>,
}

pub fn router(backend: Arc<dyn HeadlessCaptureBackend>) -> Router {
    Router::new()
        .route("/health/live", get(|| async { StatusCode::NO_CONTENT }))
        .route("/health/ready", get(ready))
        .route("/v1/headless/capture", post(capture))
        .with_state(AppState { backend })
}

async fn ready(State(state): State<AppState>) -> StatusCode {
    if state.backend.ready().await.is_ok() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn capture(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(request): Json<HeadlessCaptureRequest>,
) -> Response {
    if Url::parse(&request.url).is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid url");
    }
    if request.profile_id.trim().is_empty() || request.profile_id.len() > 256 {
        return error(StatusCode::BAD_REQUEST, "invalid profile_id");
    }
    if request.width == 0
        || request.height == 0
        || request.width > 100_000
        || request.height > 100_000
        || !request.device_scale_factor.is_finite()
        || request.device_scale_factor <= 0.0
        || request.device_scale_factor > 16.0
    {
        return error(StatusCode::BAD_REQUEST, "invalid viewport");
    }

    let frame = match state.backend.capture(request).await {
        Ok(frame) => frame,
        Err(message) => return error(StatusCode::BAD_GATEWAY, message),
    };

    let mut response = Response::new(Body::from(frame.bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&frame.content_type) {
        headers.insert(axum::http::header::CONTENT_TYPE, value);
    }
    insert(headers, "x-neroa-capture-id", &frame.capture_id);
    insert(headers, "x-neroa-renderer", &frame.renderer);
    insert(headers, "x-neroa-width", &frame.width.to_string());
    insert(headers, "x-neroa-height", &frame.height.to_string());
    insert(headers, "x-neroa-scroll-x", &frame.scroll_x.to_string());
    insert(headers, "x-neroa-scroll-y", &frame.scroll_y.to_string());
    insert(headers, "x-neroa-captured-at", &frame.captured_at);
    response
}

fn insert(headers: &mut axum::http::HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: "headless_capture",
            message: message.into(),
        }),
    )
        .into_response()
}

const fn default_width() -> u32 { 1920 }
const fn default_height() -> u32 { 1080 }
const fn default_scale() -> f64 { 1.0 }
