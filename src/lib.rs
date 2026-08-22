pub mod bridge;
pub mod compatibility;
pub mod diagnostics;
pub mod engine;
pub mod input;
pub mod lifecycle;
pub mod network;
pub mod protocol;
pub mod receipt;
pub mod router;
pub mod state_broker;
pub mod supervisor;
pub mod types;

pub use bridge::{AdaptiveBridge, BridgeConfig, BridgeError, NodeSnapshot};
pub use compatibility::{
    CompatibilityAction, CompatibilityClassifier, CompatibilityFailure, FailureClass,
};
pub use diagnostics::{BridgeDiagnosticEvent, DiagnosticRing};
pub use engine::{EngineAdapter, EngineError, LiveWebEngine};
pub use input::{MappedPoint, Ray, SpatialInputTranslator, SpatialNodeGeometry};
pub use lifecycle::{LifecyclePolicy, VisibilitySample};
pub use network::{
    NetworkAction, NetworkRouteBlock, NetworkRouter, ResourceDecision, ResourceRequest,
};
pub use receipt::{JsonlReceiptSink, MemoryReceiptSink, Receipt, ReceiptSink};
pub use router::{AdaptiveRouter, RouteDecision, RouteRule, RouterConfig};
pub use state_broker::{BrokeredState, StateBrokerError, StateContinuityBroker};
pub use supervisor::{
    BrowserLifecycleSupervisor, RuntimeRecord, SupervisorConfig, SupervisorDecision,
};
pub use types::*;
