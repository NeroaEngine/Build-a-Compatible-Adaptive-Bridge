use glam::{Mat4, Vec2, Vec3};
use neroa_compatible_adaptive_bridge::{
    AdaptiveBridge, AdaptiveRouter, BridgeConfig, EngineAdapter, LifecyclePolicy, LiveWebEngine,
    MemoryReceiptSink, PageSignals, Ray, RouteTarget, RouterConfig, SpatialInputTranslator,
    SpatialNodeGeometry, StoragePartitionId, Viewport,
};
use std::sync::Arc;
use url::Url;

#[test]
fn static_document_routes_semantic() {
    let router = AdaptiveRouter::new(RouterConfig::default());

    let signals =
        PageSignals::semantic_html(Url::parse("https://example.com/article").unwrap(), 0.97);

    let decision = router.decide(&signals);

    assert_eq!(decision.target, RouteTarget::Semantic);
}

#[test]
fn normal_interactive_page_prefers_servo() {
    let router = AdaptiveRouter::new(RouterConfig::default());

    let signals = PageSignals::live_html(Url::parse("https://example.com/app").unwrap());

    let decision = router.decide(&signals);

    assert_eq!(decision.target, RouteTarget::Servo);
}

#[test]
fn drm_escalates_to_chromium() {
    let router = AdaptiveRouter::new(RouterConfig::default());

    let mut signals = PageSignals::live_html(Url::parse("https://example.com/video").unwrap());

    signals.requirements.drm = true;

    let decision = router.decide(&signals);

    assert_eq!(decision.target, RouteTarget::Chromium);
}

#[test]
fn center_ray_maps_to_center_pixel() {
    let geometry = SpatialNodeGeometry {
        world_from_local: Mat4::IDENTITY,

        size: Vec2::new(2.0, 1.0),

        viewport: Viewport::new(2000, 1000, 1.0),
    };

    let ray = Ray::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -1.0));

    let mapped =
        SpatialInputTranslator::ray_to_browser(ray, &geometry).expect("ray should hit node");

    assert!((mapped.device_px.x - 1000.0).abs() < 0.001);

    assert!((mapped.device_px.y - 500.0).abs() < 0.001);
}

#[tokio::test]
async fn bridge_preserves_node_identity_when_promoted() {
    let servo_adapter = Arc::new(EngineAdapter::servo_scaffold());

    let chromium_adapter = Arc::new(EngineAdapter::chromium_scaffold());

    let servo: Arc<dyn LiveWebEngine> = servo_adapter.clone();

    let chromium: Arc<dyn LiveWebEngine> = chromium_adapter.clone();

    let receipts = Arc::new(MemoryReceiptSink::default());

    let bridge = AdaptiveBridge::new(
        BridgeConfig::default(),
        AdaptiveRouter::new(RouterConfig::default()),
        servo,
        chromium,
        LifecyclePolicy::default(),
        receipts.clone(),
    );

    let initial = PageSignals::live_html(Url::parse("https://example.com/app").unwrap());

    let node_id = bridge
        .open_node(
            initial,
            Viewport::new(1280, 720, 1.0),
            StoragePartitionId::new("test"),
        )
        .await
        .unwrap();

    let before = bridge.node_snapshot(node_id).unwrap();

    assert_eq!(before.route, RouteTarget::Servo);

    let mut requires_chromium =
        PageSignals::live_html(Url::parse("https://example.com/protected").unwrap());

    requires_chromium.requirements.drm = true;

    let decision = bridge.reroute(node_id, requires_chromium).await.unwrap();

    assert_eq!(decision.target, RouteTarget::Chromium);

    let after = bridge.node_snapshot(node_id).unwrap();

    assert_eq!(before.node_id, after.node_id);

    assert_eq!(after.route, RouteTarget::Chromium);

    assert_eq!(servo_adapter.view_count(), 0);

    assert_eq!(chromium_adapter.view_count(), 1);

    assert!(!receipts.entries().is_empty());
}
