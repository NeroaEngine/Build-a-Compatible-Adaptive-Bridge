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
    // NEROA_AGENT_SURFACE_V7
    //
    // Script evaluation and history traversal, awaited across the thread
    // boundary. The agent surface is built entirely on these two plus the
    // input and navigation commands that already exist.
    Evaluate {
        view_id: ViewId,
        script: String,
        reply: Reply<String>,
    },
    Traverse {
        view_id: ViewId,
        /// Negative goes back, positive goes forward.
        delta: i32,
        reply: Reply<bool>,
    },
    Reload {
        view_id: ViewId,
        reply: Reply<()>,
    },
}
