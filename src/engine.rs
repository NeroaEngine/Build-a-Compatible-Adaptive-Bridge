use crate::types::{
    ActivityState, BrowserInput, EngineCapabilities, EngineKind, PortableWebState,
    SharedGpuSurface, ViewConfig, ViewId, Viewport,
};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("view {0} was not found")]
    ViewNotFound(ViewId),

    #[error("operation is not supported: {0}")]
    Unsupported(String),

    #[error("engine failure: {0}")]
    Internal(String),
}

#[async_trait]
pub trait LiveWebEngine: Send + Sync {
    fn kind(&self) -> EngineKind;

    fn capabilities(&self) -> EngineCapabilities;

    async fn create_view(&self, config: ViewConfig) -> Result<ViewId, EngineError>;

    async fn destroy_view(&self, view_id: ViewId) -> Result<(), EngineError>;

    async fn navigate(&self, view_id: ViewId, url: Url) -> Result<(), EngineError>;

    async fn resize(&self, view_id: ViewId, viewport: Viewport) -> Result<(), EngineError>;

    async fn input(&self, view_id: ViewId, input: BrowserInput) -> Result<(), EngineError>;

    async fn set_activity(
        &self,
        view_id: ViewId,
        activity: ActivityState,
    ) -> Result<(), EngineError>;

    async fn export_state(&self, view_id: ViewId) -> Result<PortableWebState, EngineError>;

    async fn import_state(
        &self,
        view_id: ViewId,
        state: PortableWebState,
    ) -> Result<(), EngineError>;

    /// Returns a direct GPU surface lease when one is available.
    ///
    /// None does NOT mean read pixels to CPU. It means no GPU frame
    /// is currently available.
    async fn acquire_frame(&self, view_id: ViewId)
        -> Result<Option<SharedGpuSurface>, EngineError>;
}

#[derive(Clone, Debug)]
struct EngineViewState {
    config: ViewConfig,
    portable: PortableWebState,
    activity: ActivityState,
    latest_surface: Option<SharedGpuSurface>,
    last_input: Option<BrowserInput>,
}

/// Temporary concrete adapter.
///
/// This gives the rest of Neroa a real engine boundary now.
/// The Servo host and Chromium/CEF host will publish their GPU
/// surfaces and state into this seam.
pub struct EngineAdapter {
    kind: EngineKind,
    capabilities: EngineCapabilities,
    views: RwLock<HashMap<ViewId, EngineViewState>>,
}

impl EngineAdapter {
    pub fn servo_scaffold() -> Self {
        Self {
            kind: EngineKind::Servo,
            capabilities: EngineCapabilities::servo_conservative(),
            views: RwLock::new(HashMap::new()),
        }
    }

    pub fn chromium_scaffold() -> Self {
        Self {
            kind: EngineKind::Chromium,
            capabilities: EngineCapabilities::chromium_compatibility(),
            views: RwLock::new(HashMap::new()),
        }
    }

    /// Called by the actual renderer host when a GPU-backed frame is
    /// ready.
    pub fn publish_surface(
        &self,
        view_id: ViewId,
        surface: SharedGpuSurface,
    ) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        view.latest_surface = Some(surface);

        Ok(())
    }

    pub fn view_count(&self) -> usize {
        self.views.read().len()
    }
}

#[async_trait]
impl LiveWebEngine for EngineAdapter {
    fn kind(&self) -> EngineKind {
        self.kind
    }

    fn capabilities(&self) -> EngineCapabilities {
        self.capabilities.clone()
    }

    async fn create_view(&self, config: ViewConfig) -> Result<ViewId, EngineError> {
        let view_id = Uuid::new_v4();

        let portable = PortableWebState::new(config.initial_url.clone());

        self.views.write().insert(
            view_id,
            EngineViewState {
                config,
                portable,
                activity: ActivityState::Dormant,
                latest_surface: None,
                last_input: None,
            },
        );

        Ok(view_id)
    }

    async fn destroy_view(&self, view_id: ViewId) -> Result<(), EngineError> {
        if self.views.write().remove(&view_id).is_none() {
            return Err(EngineError::ViewNotFound(view_id));
        }

        Ok(())
    }

    async fn navigate(&self, view_id: ViewId, url: Url) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        // Remove forward history.
        let keep = view.portable.history_index.saturating_add(1);

        view.portable.history.truncate(keep);

        view.portable.history.push(url.clone());

        view.portable.history_index = view.portable.history.len().saturating_sub(1);

        view.portable.url = url;

        Ok(())
    }

    async fn resize(&self, view_id: ViewId, viewport: Viewport) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        view.config.viewport = viewport;

        Ok(())
    }

    async fn input(&self, view_id: ViewId, input: BrowserInput) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        if let BrowserInput::Scroll {
            delta_x, delta_y, ..
        } = &input
        {
            view.portable.scroll_x += *delta_x;
            view.portable.scroll_y += *delta_y;
        }

        view.last_input = Some(input);

        Ok(())
    }

    async fn set_activity(
        &self,
        view_id: ViewId,
        activity: ActivityState,
    ) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        view.activity = activity;

        Ok(())
    }

    async fn export_state(&self, view_id: ViewId) -> Result<PortableWebState, EngineError> {
        let views = self.views.read();

        let view = views
            .get(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        Ok(view.portable.clone())
    }

    async fn import_state(
        &self,
        view_id: ViewId,
        state: PortableWebState,
    ) -> Result<(), EngineError> {
        let mut views = self.views.write();

        let view = views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        view.portable = state;

        Ok(())
    }

    async fn acquire_frame(
        &self,
        view_id: ViewId,
    ) -> Result<Option<SharedGpuSurface>, EngineError> {
        let views = self.views.read();

        let view = views
            .get(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        Ok(view.latest_surface.clone())
    }
}
