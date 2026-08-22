use neroa_compatible_adaptive_bridge::{
    ActivityState, BrowserLifecycleSupervisor, CompatibilityAction, CompatibilityClassifier,
    CompatibilityFailure, DiagnosticRing, FailureClass, PageRequirements, RouteTarget,
    StateContinuityBroker, StoragePartitionId, SupervisorConfig, SupervisorDecision,
};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

#[test]
fn servo_failure_escalates_to_chromium() {
    let classifier = CompatibilityClassifier;
    let failure = CompatibilityFailure::new(
        FailureClass::UnsupportedFeature,
        "unsupported page feature",
        true,
    );
    assert_eq!(
        classifier.action(RouteTarget::Servo, &failure),
        CompatibilityAction::Escalate(RouteTarget::Chromium)
    );
}

#[test]
fn requirements_classifier_detects_drm() {
    let classifier = CompatibilityClassifier;
    let mut requirements = PageRequirements::default();
    requirements.drm = true;
    let failure = classifier
        .classify_requirements(&requirements)
        .expect("DRM should be classified");
    assert_eq!(failure.class, FailureClass::DrmRequired);
}

#[test]
fn state_broker_preserves_node_identity() {
    let broker = StateContinuityBroker::new();
    let node = Uuid::new_v4();
    broker.initialize(
        node,
        Url::parse("https://example.com").unwrap(),
        StoragePartitionId::new("default"),
    );
    broker
        .update_url(node, Url::parse("https://example.com/next").unwrap())
        .unwrap();
    let snapshot = broker.snapshot(node).unwrap();
    assert_eq!(snapshot.node_id, node);
    assert_eq!(snapshot.portable.url.as_str(), "https://example.com/next");
    assert_eq!(snapshot.portable.history.len(), 2);
}

#[test]
fn supervisor_escalates_stale_servo_runtime() {
    let supervisor = BrowserLifecycleSupervisor::new(SupervisorConfig {
        heartbeat_timeout: Duration::ZERO,
        dormant_destroy_after: Duration::from_secs(60),
        max_runtime_crashes: 2,
    });
    let node = Uuid::new_v4();
    supervisor.register(
        node,
        RouteTarget::Servo,
        Some(Uuid::new_v4()),
        ActivityState::Active,
    );
    std::thread::sleep(Duration::from_millis(1));
    assert_eq!(
        supervisor.inspect(node),
        SupervisorDecision::Escalate(RouteTarget::Chromium)
    );
}

#[test]
fn diagnostics_are_bounded() {
    use neroa_compatible_adaptive_bridge::BridgeDiagnosticEvent;
    let ring = DiagnosticRing::new(2);
    ring.push(BridgeDiagnosticEvent::new("one"));
    ring.push(BridgeDiagnosticEvent::new("two"));
    ring.push(BridgeDiagnosticEvent::new("three"));
    let entries = ring.snapshot();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "two");
    assert_eq!(entries[1].message, "three");
}
