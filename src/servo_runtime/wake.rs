use std::sync::Arc;

/// Thread-safe wake seam used by both bridge commands and Servo frame-ready
/// notifications to request another host event-loop turn.
pub trait ServoHostNotifier: Send + Sync {
    fn notify(&self);
}

impl<F> ServoHostNotifier for F
where
    F: Fn() + Send + Sync,
{
    fn notify(&self) {
        self();
    }
}

#[derive(Default)]
pub struct NoopServoHostNotifier;

impl ServoHostNotifier for NoopServoHostNotifier {
    fn notify(&self) {}
}

pub(crate) type SharedServoHostNotifier = Arc<dyn ServoHostNotifier>;
