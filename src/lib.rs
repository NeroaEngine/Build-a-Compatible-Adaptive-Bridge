// NEROA_AGENT_SURFACE_V7 / NEROA_TRAINING_SURFACE_V7
#[cfg(feature = "servo-runtime")]
pub mod agent;

// NEROA_OAUTH_LOOPBACK_V11
#[cfg(feature = "oauth")]
pub mod oauth;

pub mod bridge;
#[cfg(feature = "chromium-runtime")]
pub mod chromium_runtime;
pub mod compatibility;
pub mod diagnostics;
pub mod engine;
pub mod input;
pub mod jit_residency;
pub mod lifecycle;
pub mod network;
pub mod protocol;
pub mod receipt;
pub mod router;
#[cfg(feature = "servo-runtime")]
pub mod servo_runtime;
pub mod state_broker;
pub mod supervisor;
pub mod types;

pub use bridge::{AdaptiveBridge, BridgeConfig, BridgeError, NodeSnapshot};
#[cfg(feature = "chromium-runtime")]
pub use chromium_runtime::{
    ChromiumAcceleratedFrame, ChromiumBackend, ChromiumEngineProxy, ChromiumGpuFrameImporter,
    ChromiumHost, ChromiumHostNotifier, NoChromiumBackend, NoChromiumGpuFrameImporter,
    NoopChromiumHostNotifier,
};
pub use compatibility::{
    CompatibilityAction, CompatibilityClassifier, CompatibilityFailure, FailureClass,
};
pub use diagnostics::{BridgeDiagnosticEvent, DiagnosticRing};
pub use engine::{EngineAdapter, EngineError, LiveWebEngine};
pub use input::{MappedPoint, Ray, SpatialInputTranslator, SpatialNodeGeometry};
pub use jit_residency::{
    GovernedRef, HydratedObject, JitResidencyClient, JitResidencyError, JitResidencyMetrics,
    ResidencyConsumer, ResidencyScope, VaultJitTransport,
};
pub use lifecycle::{LifecyclePolicy, VisibilitySample};
pub use network::{
    NetworkAction, NetworkRouteBlock, NetworkRouter, ResourceDecision, ResourceRequest,
};
pub use receipt::{JsonlReceiptSink, MemoryReceiptSink, Receipt, ReceiptSink};
pub use router::{AdaptiveRouter, RouteDecision, RouteRule, RouterConfig};
#[cfg(feature = "servo-runtime")]
pub use servo_runtime::{NoopServoHostNotifier, ServoEngineProxy, ServoHost, ServoHostNotifier};
pub use state_broker::{BrokeredState, StateBrokerError, StateContinuityBroker};
pub use supervisor::{
    BrowserLifecycleSupervisor, RuntimeRecord, SupervisorConfig, SupervisorDecision,
};
pub use types::*;

#[cfg(feature = "servo-runtime")]
pub use servo_runtime::ServoNavigationAdapter;
