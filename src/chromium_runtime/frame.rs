use crate::engine::EngineError;
use crate::types::{SharedGpuSurface, ViewId, Viewport};

/// A temporary accelerated-paint frame published by the Chromium host.
///
/// The raw platform handle is valid only for the duration defined by the
/// Chromium/CEF callback. Implementations must import/copy it GPU-to-GPU into
/// Neroa-owned storage before the callback returns. The raw handle itself must
/// never be retained as a `SharedGpuSurface` lease.
#[derive(Clone, Copy, Debug)]
pub struct ChromiumAcceleratedFrame {
    pub raw_shared_handle: u64,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

/// Converts Chromium's callback-scoped accelerated frame into a Neroa-owned GPU
/// lease. CPU `OnPaint` pixel buffers are outside this contract by design.
pub trait ChromiumGpuFrameImporter {
    fn supports_external_gpu_surface(&self) -> bool {
        false
    }

    fn import_accelerated_frame(
        &self,
        view_id: ViewId,
        viewport: &Viewport,
        frame: ChromiumAcceleratedFrame,
    ) -> Result<Option<SharedGpuSurface>, EngineError>;
}

/// Fail-closed importer used until the real CEF/D3D accelerated path is installed.
#[derive(Default)]
pub struct NoChromiumGpuFrameImporter;

impl ChromiumGpuFrameImporter for NoChromiumGpuFrameImporter {
    fn import_accelerated_frame(
        &self,
        _view_id: ViewId,
        _viewport: &Viewport,
        _frame: ChromiumAcceleratedFrame,
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
    fn default_chromium_importer_fails_closed_without_cpu_fallback() {
        let importer = NoChromiumGpuFrameImporter;
        let viewport = Viewport::new(1280, 720, 1.0);
        let frame = ChromiumAcceleratedFrame {
            raw_shared_handle: 7,
            width: 1280,
            height: 720,
            generation: 1,
        };

        assert!(!importer.supports_external_gpu_surface());
        assert!(importer
            .import_accelerated_frame(Uuid::new_v4(), &viewport, frame)
            .expect("default Chromium importer should not fail")
            .is_none());
    }
}
