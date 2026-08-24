use std::sync::Arc;

/// Thread-safe wake seam for the Chromium compatibility host.
///
/// The bridge may enqueue commands from any thread, but CEF/Chromium objects
/// remain on their dedicated host thread. Implementations typically signal the
/// CEF message-loop integration or a host-thread event.
pub trait ChromiumHostNotifier: Send + Sync {
    fn notify(&self);
}

impl<F> ChromiumHostNotifier for F
where
    F: Fn() + Send + Sync,
{
    fn notify(&self) {
        self();
    }
}

#[derive(Default)]
pub struct NoopChromiumHostNotifier;

impl ChromiumHostNotifier for NoopChromiumHostNotifier {
    fn notify(&self) {}
}

pub(crate) type SharedChromiumHostNotifier = Arc<dyn ChromiumHostNotifier>;
