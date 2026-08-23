use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::engine::{EngineError, LiveWebEngine};
use crate::types::{
    ActivityState, BrowserInput, EngineCapabilities, EngineKind, PortableWebState,
    SharedGpuSurface, ViewConfig, ViewId, Viewport,
};

use super::command::ServoCommand;
use super::wake::SharedServoHostNotifier;

/// Renderer-independent, Send + Sync handle to a Servo host.
///
/// Servo itself is intentionally not stored here. The proxy only owns a
/// cross-thread command sender and a thread-safe host notifier; WebView,
/// RenderingContext, and Rc state remain on the Servo event-loop owner.
#[derive(Clone)]
pub struct ServoEngineProxy {
    tx: mpsc::UnboundedSender<ServoCommand>,
    notifier: SharedServoHostNotifier,
    external_gpu_surface: bool,
}

impl ServoEngineProxy {
    pub(crate) fn new(
        tx: mpsc::UnboundedSender<ServoCommand>,
        notifier: SharedServoHostNotifier,
        external_gpu_surface: bool,
    ) -> Self {
        Self {
            tx,
            notifier,
            external_gpu_surface,
        }
    }

    pub(crate) fn channel(
        notifier: SharedServoHostNotifier,
        external_gpu_surface: bool,
    ) -> (Self, mpsc::UnboundedReceiver<ServoCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self::new(tx, notifier, external_gpu_surface),
            rx,
        )
    }

    fn send(&self, command: ServoCommand) -> Result<(), EngineError> {
        self.tx.send(command).map_err(|_| {
            EngineError::Internal("Servo host command channel is closed".to_string())
        })?;
        self.notifier.notify();
        Ok(())
    }

    async fn await_reply<T>(rx: oneshot::Receiver<Result<T, EngineError>>) -> Result<T, EngineError> {
        rx.await.map_err(|_| {
            EngineError::Internal("Servo host dropped command reply".to_string())
        })?
    }
}

#[async_trait]
impl LiveWebEngine for ServoEngineProxy {
    fn kind(&self) -> EngineKind {
        EngineKind::Servo
    }

    fn capabilities(&self) -> EngineCapabilities {
        let mut capabilities = EngineCapabilities::servo_conservative();
        capabilities.external_gpu_surface = self.external_gpu_surface;
        capabilities
    }

    async fn create_view(&self, config: ViewConfig) -> Result<ViewId, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::CreateView { config, reply })?;
        Self::await_reply(rx).await
    }

    async fn destroy_view(&self, view_id: ViewId) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::DestroyView { view_id, reply })?;
        Self::await_reply(rx).await
    }

    async fn navigate(&self, view_id: ViewId, url: Url) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::Navigate {
            view_id,
            url,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn resize(&self, view_id: ViewId, viewport: Viewport) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::Resize {
            view_id,
            viewport,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn input(&self, view_id: ViewId, input: BrowserInput) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::Input {
            view_id,
            input,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn set_activity(
        &self,
        view_id: ViewId,
        activity: ActivityState,
    ) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::SetActivity {
            view_id,
            activity,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn export_state(&self, view_id: ViewId) -> Result<PortableWebState, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::ExportState { view_id, reply })?;
        Self::await_reply(rx).await
    }

    async fn import_state(
        &self,
        view_id: ViewId,
        state: PortableWebState,
    ) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::ImportState {
            view_id,
            state,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn acquire_frame(
        &self,
        view_id: ViewId,
    ) -> Result<Option<SharedGpuSurface>, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ServoCommand::AcquireFrame { view_id, reply })?;
        Self::await_reply(rx).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::servo_runtime::wake::NoopServoHostNotifier;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn servo_proxy_is_send_and_sync() {
        assert_send_sync::<ServoEngineProxy>();
    }

    #[test]
    fn proxy_channel_can_be_constructed_without_servo_objects() {
        let notifier: SharedServoHostNotifier = Arc::new(NoopServoHostNotifier);
        let (proxy, _rx) = ServoEngineProxy::channel(notifier, false);
        assert!(!proxy.capabilities().external_gpu_surface);
    }

    #[test]
    fn proxy_reports_injected_external_gpu_capability() {
        let notifier: SharedServoHostNotifier = Arc::new(NoopServoHostNotifier);
        let (proxy, _rx) = ServoEngineProxy::channel(notifier, true);
        assert!(proxy.capabilities().external_gpu_surface);
    }
}
