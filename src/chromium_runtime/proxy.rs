use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::engine::{EngineError, LiveWebEngine};
use crate::types::{
    ActivityState, BrowserInput, EngineCapabilities, EngineKind, PortableWebState,
    SharedGpuSurface, ViewConfig, ViewId, Viewport,
};

use super::command::ChromiumCommand;
use super::wake::SharedChromiumHostNotifier;

/// Renderer-independent, Send + Sync handle to the Chromium compatibility host.
///
/// Chromium/CEF objects and process-local accelerated-paint handles remain on
/// the compatibility host side of this channel. Neroa owns canonical node
/// identity and only sees the LiveWebEngine contract.
#[derive(Clone)]
pub struct ChromiumEngineProxy {
    tx: mpsc::UnboundedSender<ChromiumCommand>,
    notifier: SharedChromiumHostNotifier,
    external_gpu_surface: bool,
}

impl ChromiumEngineProxy {
    pub(crate) fn new(
        tx: mpsc::UnboundedSender<ChromiumCommand>,
        notifier: SharedChromiumHostNotifier,
        external_gpu_surface: bool,
    ) -> Self {
        Self {
            tx,
            notifier,
            external_gpu_surface,
        }
    }

    pub(crate) fn channel(
        notifier: SharedChromiumHostNotifier,
        external_gpu_surface: bool,
    ) -> (Self, mpsc::UnboundedReceiver<ChromiumCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self::new(tx, notifier, external_gpu_surface), rx)
    }

    fn send(&self, command: ChromiumCommand) -> Result<(), EngineError> {
        self.tx.send(command).map_err(|_| {
            EngineError::Internal("Chromium host command channel is closed".to_string())
        })?;
        self.notifier.notify();
        Ok(())
    }

    async fn await_reply<T>(
        rx: oneshot::Receiver<Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        rx.await
            .map_err(|_| EngineError::Internal("Chromium host dropped command reply".to_string()))?
    }
}

#[async_trait]
impl LiveWebEngine for ChromiumEngineProxy {
    fn kind(&self) -> EngineKind {
        EngineKind::Chromium
    }

    fn capabilities(&self) -> EngineCapabilities {
        let mut capabilities = EngineCapabilities::chromium_compatibility();
        capabilities.external_gpu_surface = self.external_gpu_surface;
        capabilities
    }

    async fn create_view(&self, config: ViewConfig) -> Result<ViewId, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::CreateView { config, reply })?;
        Self::await_reply(rx).await
    }

    async fn destroy_view(&self, view_id: ViewId) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::DestroyView { view_id, reply })?;
        Self::await_reply(rx).await
    }

    async fn navigate(&self, view_id: ViewId, url: Url) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::Navigate {
            view_id,
            url,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn resize(&self, view_id: ViewId, viewport: Viewport) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::Resize {
            view_id,
            viewport,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn input(&self, view_id: ViewId, input: BrowserInput) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::Input {
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
        self.send(ChromiumCommand::SetActivity {
            view_id,
            activity,
            reply,
        })?;
        Self::await_reply(rx).await
    }

    async fn export_state(&self, view_id: ViewId) -> Result<PortableWebState, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::ExportState { view_id, reply })?;
        Self::await_reply(rx).await
    }

    async fn import_state(
        &self,
        view_id: ViewId,
        state: PortableWebState,
    ) -> Result<(), EngineError> {
        let (reply, rx) = oneshot::channel();
        self.send(ChromiumCommand::ImportState {
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
        self.send(ChromiumCommand::AcquireFrame { view_id, reply })?;
        Self::await_reply(rx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn assert_send_sync<T: Send + Sync>() {}

    fn noop_notifier() -> SharedChromiumHostNotifier {
        Arc::new(|| {})
    }

    #[test]
    fn chromium_proxy_is_send_and_sync() {
        assert_send_sync::<ChromiumEngineProxy>();
    }

    #[test]
    fn chromium_proxy_can_fail_closed_on_gpu_export() {
        let (proxy, _rx) = ChromiumEngineProxy::channel(noop_notifier(), false);
        assert!(!proxy.capabilities().external_gpu_surface);
        assert_eq!(proxy.kind(), EngineKind::Chromium);
    }

    #[test]
    fn chromium_proxy_reports_accelerated_gpu_capability_when_injected() {
        let (proxy, _rx) = ChromiumEngineProxy::channel(noop_notifier(), true);
        assert!(proxy.capabilities().external_gpu_surface);
    }

    #[tokio::test]
    async fn proxy_wakes_host_after_command_enqueue() {
        let wake_count = Arc::new(AtomicUsize::new(0));
        let notifier_count = wake_count.clone();
        let notifier: SharedChromiumHostNotifier = Arc::new(move || {
            notifier_count.fetch_add(1, Ordering::SeqCst);
        });
        let (proxy, mut rx) = ChromiumEngineProxy::channel(notifier, false);

        let request = tokio::spawn(async move {
            proxy
                .destroy_view(uuid::Uuid::nil())
                .await
                .expect_err("host has not replied");
        });

        tokio::task::yield_now().await;
        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        let command = rx.recv().await.expect("command should be queued");
        command.fail("test host failure");
        request.await.expect("request task should complete");
    }
}
