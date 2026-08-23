use tokio::sync::oneshot;
use url::Url;

use crate::engine::EngineError;
use crate::types::{
    ActivityState, BrowserInput, PortableWebState, SharedGpuSurface, ViewConfig, ViewId, Viewport,
};

pub(crate) type Reply<T> = oneshot::Sender<Result<T, EngineError>>;

/// Cross-thread command contract between the renderer-independent bridge and
/// Servo's event-loop-owned host.
///
/// No Servo WebView, RenderingContext, Rc, or platform window handle crosses
/// this boundary.
pub(crate) enum ServoCommand {
    Wake,
    CreateView {
        config: ViewConfig,
        reply: Reply<ViewId>,
    },
    DestroyView {
        view_id: ViewId,
        reply: Reply<()>,
    },
    Navigate {
        view_id: ViewId,
        url: Url,
        reply: Reply<()>,
    },
    Resize {
        view_id: ViewId,
        viewport: Viewport,
        reply: Reply<()>,
    },
    Input {
        view_id: ViewId,
        input: BrowserInput,
        reply: Reply<()>,
    },
    SetActivity {
        view_id: ViewId,
        activity: ActivityState,
        reply: Reply<()>,
    },
    ExportState {
        view_id: ViewId,
        reply: Reply<PortableWebState>,
    },
    ImportState {
        view_id: ViewId,
        state: PortableWebState,
        reply: Reply<()>,
    },
    AcquireFrame {
        view_id: ViewId,
        reply: Reply<Option<SharedGpuSurface>>,
    },
}

impl ServoCommand {
    pub(crate) fn fail(self, message: impl Into<String>) {
        let message = message.into();
        match self {
            Self::Wake => {},
            Self::CreateView { reply, .. } => {
                let _ = reply.send(Err(EngineError::Internal(message)));
            },
            Self::DestroyView { reply, .. }
            | Self::Navigate { reply, .. }
            | Self::Resize { reply, .. }
            | Self::Input { reply, .. }
            | Self::SetActivity { reply, .. }
            | Self::ImportState { reply, .. } => {
                let _ = reply.send(Err(EngineError::Internal(message)));
            },
            Self::ExportState { reply, .. } => {
                let _ = reply.send(Err(EngineError::Internal(message)));
            },
            Self::AcquireFrame { reply, .. } => {
                let _ = reply.send(Err(EngineError::Internal(message)));
            },
        }
    }
}
