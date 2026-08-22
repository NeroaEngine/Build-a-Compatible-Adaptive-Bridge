use crate::types::{ActivityState, EngineKind, NodeId, RouteTarget, ViewId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeDiagnosticEvent {
    pub timestamp_unix_ms: u64,
    pub node_id: Option<NodeId>,
    pub view_id: Option<ViewId>,
    pub route: Option<RouteTarget>,
    pub engine: Option<EngineKind>,
    pub activity: Option<ActivityState>,
    pub message: String,
}

impl BridgeDiagnosticEvent {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            timestamp_unix_ms: now_ms(),
            node_id: None,
            view_id: None,
            route: None,
            engine: None,
            activity: None,
            message: message.into(),
        }
    }
    pub fn for_node(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }
    pub fn with_route(mut self, route: RouteTarget) -> Self {
        self.route = Some(route);
        self.engine = route.engine_kind();
        self
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub struct DiagnosticRing {
    capacity: usize,
    events: RwLock<VecDeque<BridgeDiagnosticEvent>>,
}

impl DiagnosticRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: RwLock::new(VecDeque::new()),
        }
    }
    pub fn push(&self, event: BridgeDiagnosticEvent) {
        let mut events = self.events.write();
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }
    pub fn snapshot(&self) -> Vec<BridgeDiagnosticEvent> {
        self.events.read().iter().cloned().collect()
    }
    pub fn len(&self) -> usize {
        self.events.read().len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.read().is_empty()
    }
}
