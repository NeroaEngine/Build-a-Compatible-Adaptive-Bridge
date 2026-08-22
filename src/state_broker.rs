use crate::types::{NodeId, PortableWebState, StoragePartitionId};
use parking_lot::RwLock;
use std::collections::HashMap;
use thiserror::Error;
use url::Url;

#[derive(Clone, Debug)]
pub struct BrokeredState {
    pub node_id: NodeId,
    pub portable: PortableWebState,
    pub storage_partition: StoragePartitionId,
    pub generation: u64,
}

#[derive(Debug, Error)]
pub enum StateBrokerError {
    #[error("no brokered state for node {0}")]
    Missing(NodeId),
}

#[derive(Default)]
pub struct StateContinuityBroker {
    states: RwLock<HashMap<NodeId, BrokeredState>>,
}

impl StateContinuityBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn initialize(
        &self,
        node_id: NodeId,
        url: Url,
        storage_partition: StoragePartitionId,
    ) -> BrokeredState {
        let state = BrokeredState {
            node_id,
            portable: PortableWebState::new(url),
            storage_partition,
            generation: 1,
        };
        self.states.write().insert(node_id, state.clone());
        state
    }

    pub fn import(
        &self,
        node_id: NodeId,
        portable: PortableWebState,
        storage_partition: StoragePartitionId,
    ) -> BrokeredState {
        let mut states = self.states.write();
        let generation = states
            .get(&node_id)
            .map(|s| s.generation.saturating_add(1))
            .unwrap_or(1);
        let state = BrokeredState {
            node_id,
            portable,
            storage_partition,
            generation,
        };
        states.insert(node_id, state.clone());
        state
    }

    pub fn snapshot(&self, node_id: NodeId) -> Result<BrokeredState, StateBrokerError> {
        self.states
            .read()
            .get(&node_id)
            .cloned()
            .ok_or(StateBrokerError::Missing(node_id))
    }

    pub fn update_url(&self, node_id: NodeId, url: Url) -> Result<BrokeredState, StateBrokerError> {
        let mut states = self.states.write();
        let state = states
            .get_mut(&node_id)
            .ok_or(StateBrokerError::Missing(node_id))?;
        let keep = state.portable.history_index.saturating_add(1);
        state.portable.history.truncate(keep);
        state.portable.history.push(url.clone());
        state.portable.history_index = state.portable.history.len().saturating_sub(1);
        state.portable.url = url;
        state.generation = state.generation.saturating_add(1);
        Ok(state.clone())
    }

    pub fn update_scroll(
        &self,
        node_id: NodeId,
        x: f64,
        y: f64,
    ) -> Result<BrokeredState, StateBrokerError> {
        let mut states = self.states.write();
        let state = states
            .get_mut(&node_id)
            .ok_or(StateBrokerError::Missing(node_id))?;
        state.portable.scroll_x = x;
        state.portable.scroll_y = y;
        state.generation = state.generation.saturating_add(1);
        Ok(state.clone())
    }

    pub fn remove(&self, node_id: NodeId) -> Option<BrokeredState> {
        self.states.write().remove(&node_id)
    }
    pub fn len(&self) -> usize {
        self.states.read().len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.read().is_empty()
    }
}
