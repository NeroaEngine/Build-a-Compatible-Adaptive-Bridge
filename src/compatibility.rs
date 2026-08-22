use crate::types::{PageRequirements, RouteTarget};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureClass {
    UnsupportedFeature,
    RendererCrash,
    NavigationFailure,
    AuthenticationFailure,
    DrmRequired,
    WebGpuRequired,
    WebRtcRequired,
    ClientCertificateRequired,
    Timeout,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibilityFailure {
    pub class: FailureClass,
    pub message: String,
    pub recoverable: bool,
}

impl CompatibilityFailure {
    pub fn new(class: FailureClass, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            class,
            message: message.into(),
            recoverable,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompatibilityAction {
    Stay,
    Retry,
    Escalate(RouteTarget),
    Fail,
}

#[derive(Clone, Debug, Default)]
pub struct CompatibilityClassifier;

impl CompatibilityClassifier {
    pub fn classify_requirements(&self, req: &PageRequirements) -> Option<CompatibilityFailure> {
        if req.drm {
            return Some(CompatibilityFailure::new(
                FailureClass::DrmRequired,
                "page requires DRM",
                true,
            ));
        }
        if req.webgpu {
            return Some(CompatibilityFailure::new(
                FailureClass::WebGpuRequired,
                "page requires WebGPU",
                true,
            ));
        }
        if req.webrtc {
            return Some(CompatibilityFailure::new(
                FailureClass::WebRtcRequired,
                "page requires WebRTC",
                true,
            ));
        }
        if req.client_certificate {
            return Some(CompatibilityFailure::new(
                FailureClass::ClientCertificateRequired,
                "page requires client certificate support",
                true,
            ));
        }
        None
    }

    pub fn action(
        &self,
        current: RouteTarget,
        failure: &CompatibilityFailure,
    ) -> CompatibilityAction {
        match current {
            RouteTarget::Semantic => {
                if failure.recoverable {
                    CompatibilityAction::Escalate(RouteTarget::Servo)
                } else {
                    CompatibilityAction::Fail
                }
            }
            RouteTarget::Servo => {
                if failure.recoverable {
                    CompatibilityAction::Escalate(RouteTarget::Chromium)
                } else {
                    CompatibilityAction::Fail
                }
            }
            RouteTarget::Chromium => match failure.class {
                FailureClass::RendererCrash | FailureClass::Timeout => CompatibilityAction::Retry,
                _ => CompatibilityAction::Fail,
            },
        }
    }
}
