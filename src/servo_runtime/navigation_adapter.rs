use std::sync::atomic::{AtomicU64, Ordering};

use url::Url;

use crate::{engine::EngineError, types::ViewId};

use super::proxy::ServoEngineProxy;

static NEXT_NAVIGATION_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Neroa's asynchronous Servo navigation boundary.
///
/// `submit()` confirms only that the navigation command entered
/// the Servo command queue. Document load/render completion is
/// deliberately handled separately by the host lifecycle.
pub struct ServoNavigationAdapter {
    proxy: ServoEngineProxy,
    view_id: ViewId,
}

impl ServoNavigationAdapter {
    pub fn new(proxy: ServoEngineProxy, view_id: ViewId) -> Self {
        Self { proxy, view_id }
    }

    pub fn submit(&self, url: Url) -> Result<(), EngineError> {
        let generation = NEXT_NAVIGATION_GENERATION
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);

        eprintln!(
            "NEROA_NAV_ADAPTER_ACCEPT generation={} url={}",
            generation, url,
        );

        self.proxy.queue_navigation(self.view_id, url.clone())?;

        eprintln!(
            "NEROA_NAV_ADAPTER_QUEUED generation={} url={}",
            generation, url,
        );

        Ok(())
    }
}
