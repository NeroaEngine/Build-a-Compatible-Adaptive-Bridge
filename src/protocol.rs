use crate::types::{
    ActivityState, BrowserInput, PortableWebState, StoragePartitionId, ViewId, Viewport,
};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

pub const CHROMIUM_BRIDGE_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolEnvelope<T> {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub body: T,
}

impl<T> ProtocolEnvelope<T> {
    pub fn new(body: T) -> Self {
        Self {
            protocol_version: CHROMIUM_BRIDGE_PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            body,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChromiumCommand {
    Hello {
        client_name: String,
    },

    CreateView {
        node_id: Uuid,
        url: Url,
        viewport: Viewport,
        storage_partition: StoragePartitionId,
    },

    DestroyView {
        view_id: ViewId,
    },

    Navigate {
        view_id: ViewId,
        url: Url,
    },

    Resize {
        view_id: ViewId,
        viewport: Viewport,
    },

    Input {
        view_id: ViewId,
        input: BrowserInput,
    },

    SetActivity {
        view_id: ViewId,
        activity: ActivityState,
    },

    ExportState {
        view_id: ViewId,
    },

    ImportState {
        view_id: ViewId,
        state: PortableWebState,
    },

    AcquireFrame {
        view_id: ViewId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChromiumReply {
    Hello {
        server_name: String,
        protocol_version: u32,
    },

    Ack,

    ViewCreated {
        view_id: ViewId,
    },

    State {
        state: PortableWebState,
    },

    Frame {
        descriptor: Option<ExternalFrameDescriptor>,
    },

    Error {
        code: String,
        message: String,
    },
}

/// Metadata for a GPU frame.
///
/// Native handles must be transferred using the platform transport:
///
/// Windows: duplicated/shared HANDLE
/// Linux: SCM_RIGHTS / dma-buf fd
/// macOS: IOSurface/Metal object handoff
///
/// Never encode the actual frame pixels into this protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalFrameDescriptor {
    pub surface_id: Uuid,
    pub width: u32,
    pub height: u32,
    pub generation: u64,

    /// Identifies an out-of-band native handle attachment.
    pub handle_token: String,

    /// Identifies an out-of-band synchronization primitive.
    pub sync_token: Option<String>,

    pub backend: ExternalFrameBackend,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExternalFrameBackend {
    VulkanDmaBuf,
    D3d12SharedTexture,
    MetalIoSurface,
    SharedOpenGlTexture,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChromiumEvent {
    FrameReady { view_id: ViewId, generation: u64 },

    NavigationChanged { view_id: ViewId, url: Url },

    TitleChanged { view_id: ViewId, title: String },

    CursorChanged { view_id: ViewId, cursor: String },

    FocusChanged { view_id: ViewId, focused: bool },

    RendererCrashed { view_id: ViewId, reason: String },

    CompatibilityFailure { view_id: ViewId, reason: String },
}
