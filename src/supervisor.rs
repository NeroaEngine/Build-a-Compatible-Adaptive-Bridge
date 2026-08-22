use crate::compatibility::{CompatibilityAction, CompatibilityClassifier, CompatibilityFailure};
use crate::types::{ActivityState, NodeId, RouteTarget, ViewId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RuntimeRecord {
    pub node_id: NodeId,
    pub route: RouteTarget,
    pub view_id: Option<ViewId>,
    pub activity: ActivityState,
    pub created_at: Instant,
    pub last_heartbeat: Instant,
    pub crash_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SupervisorDecision {
    Healthy,
    Freeze,
    Destroy,
    Retry,
    Escalate(RouteTarget),
    Fail,
}

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub heartbeat_timeout: Duration,
    pub dormant_destroy_after: Duration,
    pub max_runtime_crashes: u32,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout: Duration::from_secs(10),
            dormant_destroy_after: Duration::from_secs(90),
            max_runtime_crashes: 2,
        }
    }
}

pub struct BrowserLifecycleSupervisor {
    config: SupervisorConfig,
    classifier: CompatibilityClassifier,
    runtimes: RwLock<HashMap<NodeId, RuntimeRecord>>,
}

impl BrowserLifecycleSupervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            classifier: CompatibilityClassifier,
            runtimes: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(
        &self,
        node_id: NodeId,
        route: RouteTarget,
        view_id: Option<ViewId>,
        activity: ActivityState,
    ) {
        let now = Instant::now();
        self.runtimes.write().insert(
            node_id,
            RuntimeRecord {
                node_id,
                route,
                view_id,
                activity,
                created_at: now,
                last_heartbeat: now,
                crash_count: 0,
            },
        );
    }

    pub fn heartbeat(&self, node_id: NodeId) -> bool {
        let mut runtimes = self.runtimes.write();
        let Some(runtime) = runtimes.get_mut(&node_id) else {
            return false;
        };
        runtime.last_heartbeat = Instant::now();
        true
    }

    pub fn set_activity(&self, node_id: NodeId, activity: ActivityState) -> bool {
        let mut runtimes = self.runtimes.write();
        let Some(runtime) = runtimes.get_mut(&node_id) else {
            return false;
        };
        runtime.activity = activity;
        true
    }

    pub fn record_failure(
        &self,
        node_id: NodeId,
        failure: CompatibilityFailure,
    ) -> SupervisorDecision {
        let mut runtimes = self.runtimes.write();
        let Some(runtime) = runtimes.get_mut(&node_id) else {
            return SupervisorDecision::Fail;
        };
        runtime.crash_count = runtime.crash_count.saturating_add(1);

        if runtime.crash_count > self.config.max_runtime_crashes {
            return match runtime.route {
                RouteTarget::Servo => SupervisorDecision::Escalate(RouteTarget::Chromium),
                _ => SupervisorDecision::Fail,
            };
        }

        match self.classifier.action(runtime.route, &failure) {
            CompatibilityAction::Stay => SupervisorDecision::Healthy,
            CompatibilityAction::Retry => SupervisorDecision::Retry,
            CompatibilityAction::Escalate(target) => SupervisorDecision::Escalate(target),
            CompatibilityAction::Fail => SupervisorDecision::Fail,
        }
    }

    pub fn inspect(&self, node_id: NodeId) -> SupervisorDecision {
        let runtimes = self.runtimes.read();
        let Some(runtime) = runtimes.get(&node_id) else {
            return SupervisorDecision::Destroy;
        };
        let now = Instant::now();
        let heartbeat_age = now.saturating_duration_since(runtime.last_heartbeat);

        if heartbeat_age > self.config.heartbeat_timeout {
            return match runtime.route {
                RouteTarget::Servo => SupervisorDecision::Escalate(RouteTarget::Chromium),
                RouteTarget::Chromium => SupervisorDecision::Retry,
                RouteTarget::Semantic => SupervisorDecision::Fail,
            };
        }

        if matches!(runtime.activity, ActivityState::Dormant)
            && now.saturating_duration_since(runtime.created_at) > self.config.dormant_destroy_after
        {
            return SupervisorDecision::Destroy;
        }

        if matches!(runtime.activity, ActivityState::Frozen) {
            return SupervisorDecision::Freeze;
        }
        SupervisorDecision::Healthy
    }

    pub fn unregister(&self, node_id: NodeId) -> Option<RuntimeRecord> {
        self.runtimes.write().remove(&node_id)
    }
    pub fn len(&self) -> usize {
        self.runtimes.read().len()
    }
}
