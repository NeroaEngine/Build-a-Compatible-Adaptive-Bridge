use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

/// Residency is a performance fact, never a semantic fact.
///
/// This module deliberately contains no cache, eviction ring, backing store,
/// or graph authority. Vault owns the canonical JIT implementation. This crate
/// is only a governed client of that authority.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResidencyConsumer {
    Browser,
    Compatibility,
}

impl ResidencyConsumer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Compatibility => "compatibility",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GovernedRef {
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineage_refs: Vec<String>,
}

impl GovernedRef {
    pub fn new(reference: impl Into<String>) -> Result<Self, JitResidencyError> {
        let value = Self {
            reference: reference.into(),
            content_hash: None,
            lineage_refs: Vec::new(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn with_hash(mut self, hash: impl Into<String>) -> Result<Self, JitResidencyError> {
        let hash = hash.into();
        if hash.trim().is_empty() {
            return Err(JitResidencyError::InvalidReference(
                "content hash must not be empty".into(),
            ));
        }
        self.content_hash = Some(hash);
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), JitResidencyError> {
        let reference = self.reference.trim();
        if reference.is_empty() {
            return Err(JitResidencyError::InvalidReference(
                "reference must not be empty".into(),
            ));
        }

        if reference.chars().any(char::is_whitespace) {
            return Err(JitResidencyError::InvalidReference(format!(
                "reference contains whitespace: {}",
                self.reference
            )));
        }

        if !reference.contains(':') {
            return Err(JitResidencyError::InvalidReference(format!(
                "governed reference must be namespaced: {}",
                self.reference
            )));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HydratedObject {
    pub reference: GovernedRef,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct JitResidencyMetrics {
    pub resident_count: u64,
    pub proxy_count: u64,
    pub capacity: u64,
    pub fault_count: u64,
    pub eviction_count: u64,
    pub cache_hits: u64,
    pub resolver_reads: u64,
    pub hydration_latency_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResidencyScope {
    pub consumer: ResidencyConsumer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

impl ResidencyScope {
    pub fn browser() -> Self {
        Self {
            consumer: ResidencyConsumer::Browser,
            node_id: None,
            route: None,
        }
    }

    pub fn compatibility() -> Self {
        Self {
            consumer: ResidencyConsumer::Compatibility,
            node_id: None,
            route: None,
        }
    }

    pub fn with_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn with_route(mut self, route: impl Into<String>) -> Self {
        self.route = Some(route.into());
        self
    }
}

#[derive(Debug, Error)]
pub enum JitResidencyError {
    #[error("invalid governed reference: {0}")]
    InvalidReference(String),

    #[error("required JIT authority is unavailable: {0}")]
    AuthorityUnavailable(String),

    #[error("required governed reference was not resolved: {0}")]
    MissingRequired(String),

    #[error("JIT residency contract failure: {0}")]
    Contract(String),
}

/// Transport seam to the Vault-owned JIT Residency Contract.
///
/// Implementations may use local IPC, Neroa Wire, HTTP, a browser Worker, or
/// another governed transport. They must not implement their own eviction or
/// canonical storage semantics here.
#[async_trait]
pub trait VaultJitTransport: Send + Sync {
    async fn get(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<Option<HydratedObject>, JitResidencyError>;

    async fn get_many(
        &self,
        scope: &ResidencyScope,
        refs: &[GovernedRef],
    ) -> Result<Vec<HydratedObject>, JitResidencyError>;

    async fn traverse(
        &self,
        scope: &ResidencyScope,
        root: &GovernedRef,
        max_depth: u32,
    ) -> Result<Vec<HydratedObject>, JitResidencyError>;

    async fn prefetch(
        &self,
        scope: &ResidencyScope,
        refs: &[GovernedRef],
        depth: u32,
    ) -> Result<(), JitResidencyError>;

    async fn pin(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError>;

    async fn unpin(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError>;

    async fn mark_dirty(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError>;

    async fn release(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError>;

    async fn is_resident(
        &self,
        scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<bool, JitResidencyError>;

    async fn metrics(
        &self,
        scope: &ResidencyScope,
    ) -> Result<JitResidencyMetrics, JitResidencyError>;
}

/// Thin governed client. It intentionally owns no local residency cache.
pub struct JitResidencyClient<T: VaultJitTransport> {
    transport: Arc<T>,
    scope: ResidencyScope,
}

impl<T: VaultJitTransport> Clone for JitResidencyClient<T> {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            scope: self.scope.clone(),
        }
    }
}

impl<T: VaultJitTransport> JitResidencyClient<T> {
    pub fn new(transport: Arc<T>, scope: ResidencyScope) -> Self {
        Self { transport, scope }
    }

    pub fn scope(&self) -> &ResidencyScope {
        &self.scope
    }

    pub async fn get(
        &self,
        reference: &GovernedRef,
    ) -> Result<Option<HydratedObject>, JitResidencyError> {
        reference.validate()?;
        self.transport.get(&self.scope, reference).await
    }

    pub async fn require(
        &self,
        reference: &GovernedRef,
    ) -> Result<HydratedObject, JitResidencyError> {
        self.get(reference)
            .await?
            .ok_or_else(|| JitResidencyError::MissingRequired(reference.reference.clone()))
    }

    pub async fn get_many(
        &self,
        refs: &[GovernedRef],
    ) -> Result<Vec<HydratedObject>, JitResidencyError> {
        validate_refs(refs)?;
        self.transport.get_many(&self.scope, refs).await
    }

    pub async fn traverse(
        &self,
        root: &GovernedRef,
        max_depth: u32,
    ) -> Result<Vec<HydratedObject>, JitResidencyError> {
        root.validate()?;
        self.transport
            .traverse(&self.scope, root, max_depth)
            .await
    }

    pub async fn prefetch(
        &self,
        refs: &[GovernedRef],
        depth: u32,
    ) -> Result<(), JitResidencyError> {
        validate_refs(refs)?;
        self.transport.prefetch(&self.scope, refs, depth).await
    }

    pub async fn pin(&self, reference: &GovernedRef) -> Result<(), JitResidencyError> {
        reference.validate()?;
        self.transport.pin(&self.scope, reference).await
    }

    pub async fn unpin(&self, reference: &GovernedRef) -> Result<(), JitResidencyError> {
        reference.validate()?;
        self.transport.unpin(&self.scope, reference).await
    }

    pub async fn mark_dirty(
        &self,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError> {
        reference.validate()?;
        self.transport.mark_dirty(&self.scope, reference).await
    }

    pub async fn release(&self, reference: &GovernedRef) -> Result<(), JitResidencyError> {
        reference.validate()?;
        self.transport.release(&self.scope, reference).await
    }

    pub async fn is_resident(
        &self,
        reference: &GovernedRef,
    ) -> Result<bool, JitResidencyError> {
        reference.validate()?;
        self.transport.is_resident(&self.scope, reference).await
    }

    pub async fn metrics(&self) -> Result<JitResidencyMetrics, JitResidencyError> {
        self.transport.metrics(&self.scope).await
    }
}

fn validate_refs(refs: &[GovernedRef]) -> Result<(), JitResidencyError> {
    for reference in refs {
        reference.validate()?;
    }
    Ok(())
}
