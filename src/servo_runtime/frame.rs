use crate::engine::EngineError;
use crate::types::{SharedGpuSurface, ViewId, Viewport};

/// Host-thread frame export seam for Servo.
///
/// Implementations may expose a compositor-shareable GPU resource owned by a
/// Neroa rendering context. They must never satisfy this contract by reading
/// Servo pixels back into a CPU RGBA buffer.
pub trait ServoFrameSource {
    /// Whether this source can ever export a compositor-shareable GPU surface.
    ///
    /// This is an adapter capability, not a promise that every acquire call will
    /// immediately return a frame.
    fn supports_external_gpu_surface(&self) -> bool {
        false
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Viewport;
    use uuid::Uuid;

    #[test]
    fn default_frame_source_fails_closed_without_cpu_fallback() {
        let source = NoSharedGpuFrameSource;
        let viewport = Viewport::new(1280, 720, 1.0);

        assert!(!source.supports_external_gpu_surface());
        assert!(source
            .acquire_surface(Uuid::new_v4(), &viewport, 1)
            .expect("default frame source should not fail")
            .is_none());
    }
}
