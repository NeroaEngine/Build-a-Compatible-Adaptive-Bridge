use crate::engine::{EngineError, LiveWebEngine};
use crate::lifecycle::{LifecyclePolicy, VisibilitySample};
use crate::receipt::{Receipt, ReceiptError, ReceiptSink};
use crate::router::{AdaptiveRouter, RouteDecision};
use crate::types::{
    ActivityState, BrowserInput, EngineKind, NodeId, PageSignals, PortableWebState, RouteTarget,
    SharedGpuSurface, StoragePartitionId, ViewConfig, ViewId, Viewport,
};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// When enabled, receipt failures become bridge errors.
    ///
    /// Leave false during bootstrap; governed production can turn
    /// this on once the receipt sink is guaranteed durable.
    pub receipt_fail_closed: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            receipt_fail_closed: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("node {0} was not found")]
    NodeNotFound(NodeId),

    #[error("node {0} currently has no live browser runtime")]
    NodeNotLive(NodeId),

    #[error(transparent)]
    Engine(#[from] EngineError),

    #[error(transparent)]
    Receipt(#[from] ReceiptError),
}

#[derive(Clone, Debug)]
struct NodeRuntime {
    node_id: NodeId,
    signals: PageSignals,
    decision: RouteDecision,
    view_id: Option<ViewId>,
    viewport: Viewport,
    storage_partition: StoragePartitionId,
    activity: ActivityState,
}

#[derive(Clone, Debug)]
pub struct NodeSnapshot {
    pub node_id: NodeId,
    pub url: Url,
    pub route: RouteTarget,
    pub engine: Option<EngineKind>,
    pub view_id: Option<ViewId>,
    pub viewport: Viewport,
    pub activity: ActivityState,
    pub route_reasons: Vec<String>,
}

pub struct AdaptiveBridge {
    config: BridgeConfig,

    router: AdaptiveRouter,

    servo: Arc<dyn LiveWebEngine>,
    chromium: Arc<dyn LiveWebEngine>,

    lifecycle: LifecyclePolicy,

    receipts: Arc<dyn ReceiptSink>,

    nodes: RwLock<HashMap<NodeId, NodeRuntime>>,
}

impl AdaptiveBridge {
    pub fn new(
        config: BridgeConfig,
        router: AdaptiveRouter,
        servo: Arc<dyn LiveWebEngine>,
        chromium: Arc<dyn LiveWebEngine>,
        lifecycle: LifecyclePolicy,
        receipts: Arc<dyn ReceiptSink>,
    ) -> Self {
        assert_eq!(
            servo.kind(),
            EngineKind::Servo,
            "Servo engine slot must contain Servo adapter"
        );

        assert_eq!(
            chromium.kind(),
            EngineKind::Chromium,
            "Chromium engine slot must contain Chromium adapter"
        );

        Self {
            config,
            router,
            servo,
            chromium,
            lifecycle,
            receipts,
            nodes: RwLock::new(HashMap::new()),
        }
    }

    pub async fn open_node(
        &self,
        signals: PageSignals,
        viewport: Viewport,
        storage_partition: StoragePartitionId,
    ) -> Result<NodeId, BridgeError> {
        let node_id = Uuid::new_v4();

        let decision = self.router.decide(&signals);

        let view_id = self
            .create_runtime_if_needed(
                node_id,
                &decision,
                &signals,
                viewport.clone(),
                storage_partition.clone(),
                None,
            )
            .await?;

        let runtime = NodeRuntime {
            node_id,
            signals: signals.clone(),
            decision: decision.clone(),
            view_id,
            viewport,
            storage_partition,
            activity: ActivityState::Dormant,
        };

        self.nodes.write().insert(node_id, runtime);

        self.emit(
            node_id,
            "node_opened",
            decision.target.engine_kind(),
            Some(decision),
            json!({
                "url": signals.url.as_str()
            }),
        )?;

        Ok(node_id)
    }

    pub async fn navigate(&self, node_id: NodeId, url: Url) -> Result<(), BridgeError> {
        let snapshot = self.runtime(node_id)?;

        if let Some(view_id) = snapshot.view_id {
            let engine = self
                .engine_for(snapshot.decision.target)
                .ok_or(BridgeError::NodeNotLive(node_id))?;

            engine.navigate(view_id, url.clone()).await?;
        }

        {
            let mut nodes = self.nodes.write();

            let node = nodes
                .get_mut(&node_id)
                .ok_or(BridgeError::NodeNotFound(node_id))?;

            node.signals.url = url.clone();
        }

        self.emit(
            node_id,
            "navigation",
            snapshot.decision.target.engine_kind(),
            Some(snapshot.decision),
            json!({
                "url": url.as_str()
            }),
        )?;

        Ok(())
    }

    pub async fn send_input(
        &self,
        node_id: NodeId,
        input: BrowserInput,
    ) -> Result<(), BridgeError> {
        let runtime = self.runtime(node_id)?;

        let view_id = runtime.view_id.ok_or(BridgeError::NodeNotLive(node_id))?;

        let engine = self
            .engine_for(runtime.decision.target)
            .ok_or(BridgeError::NodeNotLive(node_id))?;

        engine.input(view_id, input.clone()).await?;

        self.emit(
            node_id,
            "input_forwarded",
            runtime.decision.target.engine_kind(),
            Some(runtime.decision),
            json!({
                "input": input
            }),
        )?;

        Ok(())
    }

    pub async fn resize(&self, node_id: NodeId, viewport: Viewport) -> Result<(), BridgeError> {
        let runtime = self.runtime(node_id)?;

        if let Some(view_id) = runtime.view_id {
            if let Some(engine) = self.engine_for(runtime.decision.target) {
                engine.resize(view_id, viewport.clone()).await?;
            }
        }

        let mut nodes = self.nodes.write();

        let node = nodes
            .get_mut(&node_id)
            .ok_or(BridgeError::NodeNotFound(node_id))?;

        node.viewport = viewport;

        Ok(())
    }

    pub async fn update_visibility(
        &self,
        node_id: NodeId,
        visibility: VisibilitySample,
    ) -> Result<ActivityState, BridgeError> {
        let target_state = self.lifecycle.desired_state(visibility);

        let runtime = self.runtime(node_id)?;

        if let Some(view_id) = runtime.view_id {
            if let Some(engine) = self.engine_for(runtime.decision.target) {
                engine.set_activity(view_id, target_state).await?;
            }
        }

        {
            let mut nodes = self.nodes.write();

            let node = nodes
                .get_mut(&node_id)
                .ok_or(BridgeError::NodeNotFound(node_id))?;

            node.activity = target_state;
        }

        self.emit(
            node_id,
            "activity_changed",
            runtime.decision.target.engine_kind(),
            Some(runtime.decision),
            json!({
                "activity": target_state
            }),
        )?;

        Ok(target_state)
    }

    /// Re-evaluate the node and, when required, swap execution
    /// backends while preserving node identity and portable state.
    pub async fn reroute(
        &self,
        node_id: NodeId,
        signals: PageSignals,
    ) -> Result<RouteDecision, BridgeError> {
        let old = self.runtime(node_id)?;

        let new_decision = self.router.decide(&signals);

        if new_decision.target == old.decision.target {
            {
                let mut nodes = self.nodes.write();

                let node = nodes
                    .get_mut(&node_id)
                    .ok_or(BridgeError::NodeNotFound(node_id))?;

                node.signals = signals;
                node.decision = new_decision.clone();
            }

            self.emit(
                node_id,
                "route_revalidated",
                new_decision.target.engine_kind(),
                Some(new_decision.clone()),
                json!({}),
            )?;

            return Ok(new_decision);
        }

        // Snapshot portable state BEFORE touching old runtime.
        let portable_state = self.export_portable_state(&old).await?;

        // Create the replacement renderer before destroying the old
        // one. This makes renderer migration failure recoverable.
        let new_view_id = self
            .create_runtime_if_needed(
                node_id,
                &new_decision,
                &signals,
                old.viewport.clone(),
                old.storage_partition.clone(),
                portable_state.clone(),
            )
            .await?;

        // Replacement exists. Old backend can now be destroyed.
        if let Some(old_view_id) = old.view_id {
            if let Some(old_engine) = self.engine_for(old.decision.target) {
                old_engine.destroy_view(old_view_id).await?;
            }
        }

        {
            let mut nodes = self.nodes.write();

            let node = nodes
                .get_mut(&node_id)
                .ok_or(BridgeError::NodeNotFound(node_id))?;

            node.signals = signals;
            node.decision = new_decision.clone();
            node.view_id = new_view_id;
        }

        self.emit(
            node_id,
            "route_changed",
            new_decision.target.engine_kind(),
            Some(new_decision.clone()),
            json!({
                "from": old.decision.target,
                "to": new_decision.target,
                "node_identity_preserved": true
            }),
        )?;

        Ok(new_decision)
    }

    pub async fn acquire_surface(
        &self,
        node_id: NodeId,
    ) -> Result<Option<SharedGpuSurface>, BridgeError> {
        let runtime = self.runtime(node_id)?;

        let Some(view_id) = runtime.view_id else {
            return Ok(None);
        };

        let Some(engine) = self.engine_for(runtime.decision.target) else {
            return Ok(None);
        };

        Ok(engine.acquire_frame(view_id).await?)
    }

    pub fn node_snapshot(&self, node_id: NodeId) -> Result<NodeSnapshot, BridgeError> {
        let node = self.runtime(node_id)?;

        Ok(NodeSnapshot {
            node_id: node.node_id,
            url: node.signals.url,
            route: node.decision.target,
            engine: node.decision.target.engine_kind(),
            view_id: node.view_id,
            viewport: node.viewport,
            activity: node.activity,
            route_reasons: node.decision.reasons,
        })
    }

    pub fn list_nodes(&self) -> Vec<NodeSnapshot> {
        self.nodes
            .read()
            .values()
            .cloned()
            .map(|node| NodeSnapshot {
                node_id: node.node_id,
                url: node.signals.url,
                route: node.decision.target,
                engine: node.decision.target.engine_kind(),
                view_id: node.view_id,
                viewport: node.viewport,
                activity: node.activity,
                route_reasons: node.decision.reasons,
            })
            .collect()
    }

    async fn export_portable_state(
        &self,
        runtime: &NodeRuntime,
    ) -> Result<Option<PortableWebState>, BridgeError> {
        let Some(view_id) = runtime.view_id else {
            return Ok(Some(PortableWebState::new(runtime.signals.url.clone())));
        };

        let Some(engine) = self.engine_for(runtime.decision.target) else {
            return Ok(None);
        };

        Ok(Some(engine.export_state(view_id).await?))
    }

    async fn create_runtime_if_needed(
        &self,
        node_id: NodeId,
        decision: &RouteDecision,
        signals: &PageSignals,
        viewport: Viewport,
        storage_partition: StoragePartitionId,
        state: Option<PortableWebState>,
    ) -> Result<Option<ViewId>, BridgeError> {
        let Some(engine) = self.engine_for(decision.target) else {
            return Ok(None);
        };

        let initial_url = state
            .as_ref()
            .map(|s| s.url.clone())
            .unwrap_or_else(|| signals.url.clone());

        let view_id = engine
            .create_view(ViewConfig {
                node_id,
                initial_url,
                viewport,
                storage_partition,
            })
            .await?;

        if let Some(state) = state {
            engine.import_state(view_id, state).await?;
        }

        Ok(Some(view_id))
    }

    fn runtime(&self, node_id: NodeId) -> Result<NodeRuntime, BridgeError> {
        self.nodes
            .read()
            .get(&node_id)
            .cloned()
            .ok_or(BridgeError::NodeNotFound(node_id))
    }

    fn engine_for(&self, target: RouteTarget) -> Option<Arc<dyn LiveWebEngine>> {
        match target {
            RouteTarget::Semantic => None,

            RouteTarget::Servo => Some(self.servo.clone()),

            RouteTarget::Chromium => Some(self.chromium.clone()),
        }
    }

    fn emit(
        &self,
        node_id: NodeId,
        event: impl Into<String>,
        engine: Option<EngineKind>,
        route: Option<RouteDecision>,
        details: serde_json::Value,
    ) -> Result<(), BridgeError> {
        let receipt = Receipt::new(node_id, event, engine, route, details);

        match self.receipts.emit(&receipt) {
            Ok(()) => Ok(()),

            Err(error) if self.config.receipt_fail_closed => Err(error.into()),

            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "bridge receipt emission failed"
                );

                Ok(())
            }
        }
    }
}
