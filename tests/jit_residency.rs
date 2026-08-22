use async_trait::async_trait;
use neroa_compatible_adaptive_bridge::{
    GovernedRef, HydratedObject, JitResidencyClient, JitResidencyError, JitResidencyMetrics,
    ResidencyScope, VaultJitTransport,
};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
struct FakeVaultTransport {
    objects: Mutex<HashMap<String, HydratedObject>>,
    pinned: Mutex<HashSet<String>>,
    released: Mutex<Vec<String>>,
    metrics: Mutex<JitResidencyMetrics>,
}

impl FakeVaultTransport {
    fn insert(&self, object: HydratedObject) {
        self.objects
            .lock()
            .insert(object.reference.reference.clone(), object);
    }
}

#[async_trait]
impl VaultJitTransport for FakeVaultTransport {
    async fn get(
        &self,
        _scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<Option<HydratedObject>, JitResidencyError> {
        let mut metrics = self.metrics.lock();
        metrics.fault_count = metrics.fault_count.saturating_add(1);
        Ok(self.objects.lock().get(&reference.reference).cloned())
    }

    async fn get_many(
        &self,
        _scope: &ResidencyScope,
        refs: &[GovernedRef],
    ) -> Result<Vec<HydratedObject>, JitResidencyError> {
        let objects = self.objects.lock();
        Ok(refs
            .iter()
            .filter_map(|reference| objects.get(&reference.reference).cloned())
            .collect())
    }

    async fn traverse(
        &self,
        _scope: &ResidencyScope,
        root: &GovernedRef,
        _max_depth: u32,
    ) -> Result<Vec<HydratedObject>, JitResidencyError> {
        Ok(self
            .objects
            .lock()
            .get(&root.reference)
            .cloned()
            .into_iter()
            .collect())
    }

    async fn prefetch(
        &self,
        _scope: &ResidencyScope,
        _refs: &[GovernedRef],
        _depth: u32,
    ) -> Result<(), JitResidencyError> {
        Ok(())
    }

    async fn pin(
        &self,
        _scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError> {
        self.pinned.lock().insert(reference.reference.clone());
        Ok(())
    }

    async fn unpin(
        &self,
        _scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError> {
        self.pinned.lock().remove(&reference.reference);
        Ok(())
    }

    async fn mark_dirty(
        &self,
        _scope: &ResidencyScope,
        _reference: &GovernedRef,
    ) -> Result<(), JitResidencyError> {
        Ok(())
    }

    async fn release(
        &self,
        _scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<(), JitResidencyError> {
        self.released.lock().push(reference.reference.clone());
        Ok(())
    }

    async fn is_resident(
        &self,
        _scope: &ResidencyScope,
        reference: &GovernedRef,
    ) -> Result<bool, JitResidencyError> {
        Ok(self.objects.lock().contains_key(&reference.reference))
    }

    async fn metrics(
        &self,
        _scope: &ResidencyScope,
    ) -> Result<JitResidencyMetrics, JitResidencyError> {
        Ok(self.metrics.lock().clone())
    }
}

#[test]
fn governed_refs_must_be_namespaced() {
    let invalid = GovernedRef::new("raw-memory-id");
    assert!(matches!(invalid, Err(JitResidencyError::InvalidReference(_))));

    let valid = GovernedRef::new("memory:source:hash").unwrap();
    assert_eq!(valid.reference, "memory:source:hash");
}

#[tokio::test]
async fn required_hydration_fails_closed_when_authority_has_no_object() {
    let transport = Arc::new(FakeVaultTransport::default());
    let client = JitResidencyClient::new(transport, ResidencyScope::compatibility());
    let reference = GovernedRef::new("compat:profile:abc123").unwrap();

    let result = client.require(&reference).await;
    assert!(matches!(result, Err(JitResidencyError::MissingRequired(_))));
}

#[tokio::test]
async fn browser_client_hydrates_through_vault_transport_and_exposes_metrics() {
    let transport = Arc::new(FakeVaultTransport::default());
    let reference = GovernedRef::new("compat:site:legacy-portal")
        .unwrap()
        .with_hash("sha256:deadbeef")
        .unwrap();

    transport.insert(HydratedObject {
        reference: reference.clone(),
        payload: json!({
            "preferred_backend": "chromium",
            "legacy_mode": false
        }),
    });

    let client = JitResidencyClient::new(
        transport.clone(),
        ResidencyScope::browser().with_node("node-1"),
    );

    let object = client.require(&reference).await.unwrap();
    assert_eq!(object.reference, reference);
    assert_eq!(object.payload["preferred_backend"], "chromium");

    client.pin(&reference).await.unwrap();
    assert!(transport.pinned.lock().contains(&reference.reference));

    client.release(&reference).await.unwrap();
    assert_eq!(transport.released.lock().as_slice(), &[reference.reference]);

    let metrics = client.metrics().await.unwrap();
    assert_eq!(metrics.fault_count, 1);
}

#[tokio::test]
async fn hydration_result_is_independent_of_local_residency_policy() {
    let transport = Arc::new(FakeVaultTransport::default());
    let reference = GovernedRef::new("compat:rules:example").unwrap();

    transport.insert(HydratedObject {
        reference: reference.clone(),
        payload: json!({"engine":"servo","fallback":"chromium"}),
    });

    let browser = JitResidencyClient::new(
        transport.clone(),
        ResidencyScope::browser().with_route("servo"),
    );
    let compatibility = JitResidencyClient::new(
        transport,
        ResidencyScope::compatibility().with_route("chromium"),
    );

    let a = browser.require(&reference).await.unwrap();
    let b = compatibility.require(&reference).await.unwrap();

    assert_eq!(a.payload, b.payload);
}
