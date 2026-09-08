#![cfg(all(feature = "headless-http", feature = "servo-runtime"))]

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

use crate::{
    headless_observation::SpatialPageObservation,
    headless_session::{HeadlessProfileSnapshot, HeadlessSessionManager},
    types::Viewport,
};

#[derive(Clone)]
pub struct HeadlessAgentHttpState {
    sessions: HeadlessSessionManager,
    service_token: Option<Arc<str>>,
}

impl HeadlessAgentHttpState {
    #[must_use]
    pub fn new(sessions: HeadlessSessionManager, service_token: Option<String>) -> Self {
        Self {
            sessions,
            service_token: service_token.map(Arc::<str>::from),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ObserveRequest {
    pub url: String,
    pub profile_id: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_scale")]
    pub device_scale_factor: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActionRequest {
    pub profile_id: String,
    pub action: HeadlessAction,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HeadlessAction {
    Navigate { url: String },
    Click { selector: String },
    TypeText { selector: String, text: String },
    Submit { selector: String },
    WaitFor { selector: String, timeout_ms: Option<u64> },
    Reload,
    Back,
    Forward,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

pub fn router(state: HeadlessAgentHttpState) -> Router {
    Router::new()
        .route("/health/live", get(|| async { StatusCode::NO_CONTENT }))
        .route("/health/ready", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/headless/observe", post(observe))
        .route("/v1/headless/action", post(action))
        .route("/v1/headless/profiles", get(list_profiles))
        .route("/v1/headless/profiles/{profile_id}", delete(close_profile))
        .with_state(state)
}

async fn observe(
    State(state): State<HeadlessAgentHttpState>,
    headers: HeaderMap,
    Json(request): Json<ObserveRequest>,
) -> Result<Json<SpatialPageObservation>, Response> {
    authorize(&state, &headers)?;
    let url = Url::parse(&request.url)
        .map_err(|error| bad_request(format!("invalid url: {error}")))?;
    let viewport = Viewport::new(request.width, request.height, request.device_scale_factor);

    state
        .sessions
        .open_or_reuse(&request.profile_id, url, viewport)
        .await
        .map_err(engine_error)?;

    let surface = state
        .sessions
        .agent_surface(&request.profile_id)
        .await
        .map_err(engine_error)?;

    surface
        .observe_spatial()
        .await
        .map(Json)
        .map_err(engine_error)
}

async fn action(
    State(state): State<HeadlessAgentHttpState>,
    headers: HeaderMap,
    Json(request): Json<ActionRequest>,
) -> Result<Json<ActionResponse>, Response> {
    authorize(&state, &headers)?;
    let surface = state
        .sessions
        .agent_surface(&request.profile_id)
        .await
        .map_err(engine_error)?;

    let response = match request.action {
        HeadlessAction::Navigate { url } => {
            let url = Url::parse(&url)
                .map_err(|error| bad_request(format!("invalid url: {error}")))?;
            let ok = surface
                .navigate_and_wait(url, std::time::Duration::from_secs(30))
                .await
                .map_err(engine_error)?;
            ActionResponse { ok, detail: None }
        }
        HeadlessAction::Click { selector } => {
            let result = surface.click(&selector).await.map_err(engine_error)?;
            ActionResponse { ok: result.ok, detail: result.detail }
        }
        HeadlessAction::TypeText { selector, text } => {
            let result = surface
                .type_text(&selector, &text)
                .await
                .map_err(engine_error)?;
            ActionResponse { ok: result.ok, detail: result.detail }
        }
        HeadlessAction::Submit { selector } => {
            let result = surface.submit(&selector).await.map_err(engine_error)?;
            ActionResponse { ok: result.ok, detail: result.detail }
        }
        HeadlessAction::WaitFor { selector, timeout_ms } => {
            let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(10_000).min(120_000));
            let ok = surface.wait_for(&selector, timeout).await.map_err(engine_error)?;
            ActionResponse { ok, detail: None }
        }
        HeadlessAction::Reload => {
            surface.reload().await.map_err(engine_error)?;
            let ok = surface
                .wait_for_load(std::time::Duration::from_secs(30))
                .await
                .map_err(engine_error)?;
            ActionResponse { ok, detail: None }
        }
        HeadlessAction::Back => ActionResponse {
            ok: surface.back().await.map_err(engine_error)?,
            detail: None,
        },
        HeadlessAction::Forward => ActionResponse {
            ok: surface.forward().await.map_err(engine_error)?,
            detail: None,
        },
    };

    Ok(Json(response))
}

async fn list_profiles(
    State(state): State<HeadlessAgentHttpState>,
    headers: HeaderMap,
) -> Result<Json<Vec<HeadlessProfileSnapshot>>, Response> {
    authorize(&state, &headers)?;
    Ok(Json(state.sessions.list().await))
}

async fn close_profile(
    State(state): State<HeadlessAgentHttpState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<StatusCode, Response> {
    authorize(&state, &headers)?;
    if state.sessions.close(&profile_id).await.map_err(engine_error)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

fn authorize(state: &HeadlessAgentHttpState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = &state.service_token else {
        return Ok(());
    };

    let supplied = headers
        .get("x-neroa-service-token")
        .and_then(|value| value.to_str().ok());

    if supplied.is_some_and(|value| constant_time_eq(value.as_bytes(), expected.as_bytes())) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "unauthorized",
                message: "invalid headless service token".to_owned(),
            }),
        )
            .into_response())
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

fn bad_request(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            error: "bad_request",
            message,
        }),
    )
        .into_response()
}

fn engine_error(error: crate::engine::EngineError) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(ErrorBody {
            error: "browser_engine",
            message: error.to_string(),
        }),
    )
        .into_response()
}

const fn default_width() -> u32 { 1920 }
const fn default_height() -> u32 { 1080 }
const fn default_scale() -> f32 { 1.0 }
