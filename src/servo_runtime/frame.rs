use crate::engine::EngineError;
use crate::types::{SharedGpuSurface, ViewId, Viewport};

/// Host-thread frame export seam for Servo.
///
/// Implementations may expose a compositor-shareable GPU resource owned by a
/// Neroa rendering context. They must never satisfy this contract by reading
/// Servo pixels back into a CPU RGBA buffer.
pub trait ServoFrameSource {
    fn acquire_surface(
        &self,
        view_id: ViewId,
        viewport: &Viewport,
        generation: u64,
    ) -> Result<Option<SharedGpuSurface>, EngineError>;
}

/// Fail-closed default used until NeroaRenderingContext can export a real
/// compositor-shareable GPU resource.
#[derive(Default)]
pub struct NoSharedGpuFrameSource;

impl ServoFrameSource for NoSharedGpuFrameSource {
    fn acquire_surface(
        &self,
        _view_id: ViewId,
        _viewport: &Viewport,
        _generation: u64,
    ) -> Result<Option<SharedGpuSurface>, EngineError> {
        Ok(None)
    }
}
