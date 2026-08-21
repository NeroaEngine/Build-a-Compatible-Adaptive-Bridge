use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

pub type NodeId = Uuid;
pub type ViewId = Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Servo,
    Chromium,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RouteTarget {
    Semantic,
    Servo,
    Chromium,
}

impl RouteTarget {
    pub fn engine_kind(self) -> Option<EngineKind> {
        match self {
            Self::Semantic => None,
            Self::Servo => Some(EngineKind::Servo),
            Self::Chromium => Some(EngineKind::Chromium),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f32,
}

impl Viewport {
    pub fn new(width: u32, height: u32, device_scale_factor: f32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            device_scale_factor: device_scale_factor.max(0.1),
        }
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }

    pub fn css_width(&self) -> f64 {
        self.width as f64 / self.device_scale_factor as f64
    }

    pub fn css_height(&self) -> f64 {
        self.height as f64 / self.device_scale_factor as f64
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct StoragePartitionId(pub String);

impl StoragePartitionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn ephemeral() -> Self {
        Self(format!("ephemeral:{}", Uuid::new_v4()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewConfig {
    pub node_id: NodeId,
    pub initial_url: Url,
    pub viewport: Viewport,
    pub storage_partition: StoragePartitionId,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActivityState {
    Dormant,
    Frozen,
    Throttled { max_fps: u16 },
    Active,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self::Dormant
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PageRequirements {
    pub webgpu: bool,
    pub drm: bool,
    pub webauthn: bool,
    pub webrtc: bool,
    pub service_workers: bool,
    pub websocket: bool,
    pub extension_api: bool,
    pub client_certificate: bool,
}

impl PageRequirements {
    pub fn needs_live_runtime(&self) -> bool {
        self.webgpu
            || self.drm
            || self.webauthn
            || self.webrtc
            || self.service_workers
            || self.websocket
            || self.extension_api
            || self.client_certificate
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageSignals {
    pub url: Url,

    pub mime_type: Option<String>,

    /// Confidence that the document can be represented as a Neroa
    /// semantic graph without requiring browser execution.
    pub semantic_confidence: f32,

    /// True when the user expects browser-native interaction.
    pub interaction_required: bool,

    /// True when execution of application JavaScript is required.
    pub script_required: bool,

    /// True when continuous media playback is required.
    pub media_playback: bool,

    /// Requirements discovered by capability inspection.
    pub requirements: PageRequirements,

    /// Explicit rule override.
    pub force_route: Option<RouteTarget>,
}

impl PageSignals {
    pub fn html(url: Url) -> Self {
        Self {
            url,
            mime_type: Some("text/html".into()),
            semantic_confidence: 0.0,
            interaction_required: false,
            script_required: false,
            media_playback: false,
            requirements: PageRequirements::default(),
            force_route: None,
        }
    }

    pub fn semantic_html(url: Url, confidence: f32) -> Self {
        let mut signals = Self::html(url);
        signals.semantic_confidence = confidence;
        signals
    }

    pub fn live_html(url: Url) -> Self {
        let mut signals = Self::html(url);
        signals.interaction_required = true;
        signals.script_required = true;
        signals
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub offscreen_rendering: bool,

    /// Whether the engine adapter can expose a compositor-shareable
    /// GPU resource without normal-frame CPU pixel readback.
    pub external_gpu_surface: bool,

    pub webgpu: bool,
    pub drm: bool,
    pub webauthn: bool,
    pub webrtc: bool,
    pub service_workers: bool,
    pub websocket: bool,
    pub extension_api: bool,
    pub client_certificate: bool,
}

impl EngineCapabilities {
    /// Conservative Servo profile.
    ///
    /// We deliberately do not promise capabilities here until the
    /// concrete Servo host proves them at runtime.
    pub fn servo_conservative() -> Self {
        Self {
            offscreen_rendering: true,
            external_gpu_surface: false,
            webgpu: false,
            drm: false,
            webauthn: false,
            webrtc: false,
            service_workers: true,
            websocket: true,
            extension_api: false,
            client_certificate: false,
        }
    }

    /// Chromium is the compatibility backend.
    ///
    /// These are adapter promises, not Neroa architectural
    /// dependencies.
    pub fn chromium_compatibility() -> Self {
        Self {
            offscreen_rendering: true,
            external_gpu_surface: true,
            webgpu: true,
            drm: true,
            webauthn: true,
            webrtc: true,
            service_workers: true,
            websocket: true,
            extension_api: false,
            client_certificate: true,
        }
    }

    pub fn supports(&self, req: &PageRequirements) -> bool {
        (!req.webgpu || self.webgpu)
            && (!req.drm || self.drm)
            && (!req.webauthn || self.webauthn)
            && (!req.webrtc || self.webrtc)
            && (!req.service_workers || self.service_workers)
            && (!req.websocket || self.websocket)
            && (!req.extension_api || self.extension_api)
            && (!req.client_certificate || self.client_certificate)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortableWebState {
    pub url: Url,
    pub history: Vec<Url>,
    pub history_index: usize,

    /// CSS pixels.
    pub scroll_x: f64,

    /// CSS pixels.
    pub scroll_y: f64,
}

impl PortableWebState {
    pub fn new(url: Url) -> Self {
        Self {
            history: vec![url.clone()],
            history_index: 0,
            url,
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SurfaceFormat {
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba16Float,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GpuApi {
    Vulkan,
    D3d12,
    Metal,
    OpenGl,
}

#[derive(Clone, Debug)]
pub enum ExternalTextureHandle {
    /// Producer-owned Vulkan external-memory token.
    Vulkan { memory_token: u64, image_token: u64 },

    /// D3D12 shared resource HANDLE represented inside the
    /// platform adapter.
    D3d12 { shared_handle: u64 },

    /// macOS IOSurface identifier.
    Metal { io_surface_id: u32 },

    /// Same-process/shared-context GL texture.
    OpenGl { texture: u32, context_token: u64 },
}

#[derive(Clone, Debug)]
pub enum GpuSyncHandle {
    None,

    VulkanTimeline { semaphore_token: u64, value: u64 },

    D3d12Fence { fence_handle: u64, value: u64 },

    MetalSharedEvent { event_token: u64, value: u64 },

    GlFence { sync_token: u64 },
}

/// A frame is a GPU resource lease.
///
/// Normal rendering must consume this directly. Do not introduce a
/// CPU RGBA buffer into this contract.
#[derive(Clone, Debug)]
pub struct SharedGpuSurface {
    pub surface_id: Uuid,
    pub api: GpuApi,
    pub width: u32,
    pub height: u32,
    pub format: SurfaceFormat,
    pub generation: u64,
    pub texture: ExternalTextureHandle,
    pub ready: GpuSyncHandle,
    pub release: GpuSyncHandle,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct BrowserPoint {
    pub x: f64,
    pub y: f64,
}

impl BrowserPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScrollMode {
    Pixel,
    Line,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BrowserInput {
    PointerMove {
        position: BrowserPoint,
        modifiers: Modifiers,
    },

    PointerButton {
        position: BrowserPoint,
        button: MouseButton,
        state: ButtonState,
        modifiers: Modifiers,
    },

    Scroll {
        position: BrowserPoint,
        delta_x: f64,
        delta_y: f64,
        mode: ScrollMode,
        modifiers: Modifiers,
    },

    Key {
        physical_code: String,
        logical_key: String,
        state: ButtonState,
        modifiers: Modifiers,
    },

    Text {
        text: String,
    },

    Focus {
        focused: bool,
    },
}
