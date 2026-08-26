use std::collections::HashMap;
use std::rc::Rc;

use tokio::sync::mpsc;
use url::Url;
use uuid::Uuid;

use crate::engine::EngineError;
use crate::types::{
    ActivityState, BrowserInput, PortableWebState, SharedGpuSurface, ViewConfig, ViewId, Viewport,
};

use super::command::ChromiumCommand;
use super::frame::{
    ChromiumAcceleratedFrame, ChromiumGpuFrameImporter, NoChromiumGpuFrameImporter,
};
use super::proxy::ChromiumEngineProxy;
use super::wake::{ChromiumHostNotifier, SharedChromiumHostNotifier};

/// CEF/Chromium-specific browser operations owned by the compatibility host.
///
/// A concrete CEF implementation will live behind this trait. Neroa's bridge
/// never receives CEF objects, temporary accelerated-paint handles, or native
/// browser-window ownership through this boundary.
pub trait ChromiumBackend {
    fn create_view(&mut self, view_id: ViewId, config: &ViewConfig) -> Result<(), EngineError>;
    fn destroy_view(&mut self, view_id: ViewId) -> Result<(), EngineError>;
    fn navigate(&mut self, view_id: ViewId, url: &Url) -> Result<(), EngineError>;
    fn resize(&mut self, view_id: ViewId, viewport: &Viewport) -> Result<(), EngineError>;
    fn input(&mut self, view_id: ViewId, input: &BrowserInput) -> Result<(), EngineError>;
    fn set_activity(&mut self, view_id: ViewId, activity: ActivityState)
        -> Result<(), EngineError>;
}

/// Fail-closed backend used before the concrete CEF host is installed.
///
/// It deliberately refuses live Chromium work rather than silently behaving as
/// an in-memory fake browser.
#[derive(Default)]
pub struct NoChromiumBackend;

impl ChromiumBackend for NoChromiumBackend {
    fn create_view(&mut self, _view_id: ViewId, _config: &ViewConfig) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "Chromium/CEF backend is not installed".into(),
        ))
    }

    fn destroy_view(&mut self, _view_id: ViewId) -> Result<(), EngineError> {
        Ok(())
    }

    fn navigate(&mut self, _view_id: ViewId, _url: &Url) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "Chromium/CEF backend is not installed".into(),
        ))
    }

    fn resize(&mut self, _view_id: ViewId, _viewport: &Viewport) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "Chromium/CEF backend is not installed".into(),
        ))
    }

    fn input(&mut self, _view_id: ViewId, _input: &BrowserInput) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "Chromium/CEF backend is not installed".into(),
        ))
    }

    fn set_activity(
        &mut self,
        _view_id: ViewId,
        _activity: ActivityState,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "Chromium/CEF backend is not installed".into(),
        ))
    }
}

#[derive(Clone)]
struct HostView {
    config: ViewConfig,
    portable: PortableWebState,
    activity: ActivityState,
    latest_surface: Option<SharedGpuSurface>,
    accelerated_generation: u64,
}

/// Dedicated compatibility-host state machine.
///
/// This type is intentionally not Send/Sync. A concrete CEF integration owns
/// it on the Chromium host thread/message loop. `ChromiumEngineProxy` remains
/// the only cross-thread bridge object.
pub struct ChromiumHost {
    backend: Box<dyn ChromiumBackend>,
    frame_importer: Rc<dyn ChromiumGpuFrameImporter>,
    rx: mpsc::UnboundedReceiver<ChromiumCommand>,
    notifier: SharedChromiumHostNotifier,
    views: HashMap<ViewId, HostView>,
}

impl ChromiumHost {
    pub fn attach(
        backend: Box<dyn ChromiumBackend>,
        notifier: std::sync::Arc<dyn ChromiumHostNotifier>,
    ) -> (ChromiumEngineProxy, Self) {
        Self::attach_with_frame_importer(backend, notifier, Rc::new(NoChromiumGpuFrameImporter))
    }

    pub fn attach_with_frame_importer(
        backend: Box<dyn ChromiumBackend>,
        notifier: std::sync::Arc<dyn ChromiumHostNotifier>,
        frame_importer: Rc<dyn ChromiumGpuFrameImporter>,
    ) -> (ChromiumEngineProxy, Self) {
        let external_gpu_surface = frame_importer.supports_external_gpu_surface();
        let (proxy, rx) = ChromiumEngineProxy::channel(notifier.clone(), external_gpu_surface);
        (
            proxy,
            Self {
                backend,
                frame_importer,
                rx,
                notifier,
                views: HashMap::new(),
            },
        )
    }

    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Drain all pending renderer-independent commands on the Chromium host
    /// thread. The concrete CEF message-loop integration decides when this is
    /// called in response to the notifier.
    pub fn drain_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            self.handle(command);
        }
    }

    /// Entry point for CEF `OnAcceleratedPaint` integration.
    ///
    /// The callback-scoped CEF handle must be imported/copied GPU-to-GPU by the
    /// configured importer before this method returns. Only the resulting
    /// Neroa-owned `SharedGpuSurface` is retained.
    pub fn accelerated_frame(
        &mut self,
        view_id: ViewId,
        frame: ChromiumAcceleratedFrame,
    ) -> Result<(), EngineError> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;

        let generation = frame.generation;
        let surface =
            self.frame_importer
                .import_accelerated_frame(view_id, &view.config.viewport, frame)?;

        if let Some(surface) = surface {
            view.latest_surface = Some(surface);
            view.accelerated_generation = generation;
            self.notifier.notify();
        }

        Ok(())
    }

    pub fn accelerated_generation(&self, view_id: ViewId) -> Result<u64, EngineError> {
        self.views
            .get(&view_id)
            .map(|view| view.accelerated_generation)
            .ok_or(EngineError::ViewNotFound(view_id))
    }

    fn handle(&mut self, command: ChromiumCommand) {
        match command {
            ChromiumCommand::CreateView { config, reply } => {
                let view_id = Uuid::new_v4();
                let result = self.backend.create_view(view_id, &config).map(|_| {
                    self.views.insert(
                        view_id,
                        HostView {
                            portable: PortableWebState::new(config.initial_url.clone()),
                            config,
                            activity: ActivityState::Dormant,
                            latest_surface: None,
                            accelerated_generation: 0,
                        },
                    );
                    view_id
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::DestroyView { view_id, reply } => {
                let result = if self.views.contains_key(&view_id) {
                    self.backend.destroy_view(view_id).map(|_| {
                        self.views.remove(&view_id);
                    })
                } else {
                    Err(EngineError::ViewNotFound(view_id))
                };
                let _ = reply.send(result);
            }
            ChromiumCommand::Navigate {
                view_id,
                url,
                reply,
            } => {
                let result = self.with_view_backend(view_id, |backend, view| {
                    backend.navigate(view_id, &url)?;
                    let keep = view.portable.history_index.saturating_add(1);
                    view.portable.history.truncate(keep);
                    view.portable.history.push(url.clone());
                    view.portable.history_index = view.portable.history.len().saturating_sub(1);
                    view.portable.url = url;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::Resize {
                view_id,
                viewport,
                reply,
            } => {
                let result = self.with_view_backend(view_id, |backend, view| {
                    backend.resize(view_id, &viewport)?;
                    view.config.viewport = viewport;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::Input {
                view_id,
                input,
                reply,
            } => {
                let result = self.with_view_backend(view_id, |backend, view| {
                    backend.input(view_id, &input)?;
                    if let BrowserInput::Scroll {
                        delta_x, delta_y, ..
                    } = input
                    {
                        view.portable.scroll_x += delta_x;
                        view.portable.scroll_y += delta_y;
                    }
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::SetActivity {
                view_id,
                activity,
                reply,
            } => {
                let result = self.with_view_backend(view_id, |backend, view| {
                    backend.set_activity(view_id, activity)?;
                    view.activity = activity;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::ExportState { view_id, reply } => {
                let result = self
                    .views
                    .get(&view_id)
                    .map(|view| view.portable.clone())
                    .ok_or(EngineError::ViewNotFound(view_id));
                let _ = reply.send(result);
            }
            ChromiumCommand::ImportState {
                view_id,
                state,
                reply,
            } => {
                let result = self.with_view_backend(view_id, |backend, view| {
                    backend.navigate(view_id, &state.url)?;
                    view.portable = state;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ChromiumCommand::AcquireFrame { view_id, reply } => {
                let result = self
                    .views
                    .get(&view_id)
                    .map(|view| view.latest_surface.clone())
                    .ok_or(EngineError::ViewNotFound(view_id));
                let _ = reply.send(result);
            }
        }
    }

    fn with_view_backend<T>(
        &mut self,
        view_id: ViewId,
        operation: impl FnOnce(&mut dyn ChromiumBackend, &mut HostView) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;
        operation(self.backend.as_mut(), view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LiveWebEngine;
    use crate::types::{EngineKind, StoragePartitionId};

    #[derive(Default)]
    struct RecordingBackend;

    impl ChromiumBackend for RecordingBackend {
        fn create_view(
            &mut self,
            _view_id: ViewId,
            _config: &ViewConfig,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        fn destroy_view(&mut self, _view_id: ViewId) -> Result<(), EngineError> {
            Ok(())
        }
        fn navigate(&mut self, _view_id: ViewId, _url: &Url) -> Result<(), EngineError> {
            Ok(())
        }
        fn resize(&mut self, _view_id: ViewId, _viewport: &Viewport) -> Result<(), EngineError> {
            Ok(())
        }
        fn input(&mut self, _view_id: ViewId, _input: &BrowserInput) -> Result<(), EngineError> {
            Ok(())
        }
        fn set_activity(
            &mut self,
            _view_id: ViewId,
            _activity: ActivityState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    fn config() -> ViewConfig {
        ViewConfig {
            node_id: Uuid::new_v4(),
            initial_url: Url::parse("https://example.test/").unwrap(),
            viewport: Viewport::new(1280, 720, 1.0),
            storage_partition: StoragePartitionId::ephemeral(),
        }
    }

    #[tokio::test]
    async fn host_consumes_create_and_export_state_commands() {
        let (proxy, mut host) =
            ChromiumHost::attach(Box::new(RecordingBackend), std::sync::Arc::new(|| {}));
        assert_eq!(proxy.kind(), EngineKind::Chromium);

        let create = tokio::spawn({
            let proxy = proxy.clone();
            async move { proxy.create_view(config()).await }
        });
        tokio::task::yield_now().await;
        host.drain_commands();
        let view_id = create.await.unwrap().unwrap();

        let export = tokio::spawn({
            let proxy = proxy.clone();
            async move { proxy.export_state(view_id).await }
        });
        tokio::task::yield_now().await;
        host.drain_commands();
        let state = export.await.unwrap().unwrap();

        assert_eq!(state.url.as_str(), "https://example.test/");
        assert_eq!(host.view_count(), 1);
    }

    #[tokio::test]
    async fn default_host_gpu_path_fails_closed() {
        let (proxy, mut host) =
            ChromiumHost::attach(Box::new(RecordingBackend), std::sync::Arc::new(|| {}));
        let create = tokio::spawn({
            let proxy = proxy.clone();
            async move { proxy.create_view(config()).await }
        });
        tokio::task::yield_now().await;
        host.drain_commands();
        let view_id = create.await.unwrap().unwrap();

        let frame = tokio::spawn(async move { proxy.acquire_frame(view_id).await });
        tokio::task::yield_now().await;
        host.drain_commands();
        assert!(frame.await.unwrap().unwrap().is_none());
    }
}
