use glam::{Mat4, Vec2, Vec3};
use neroa_compatible_adaptive_bridge::{
    AdaptiveBridge, AdaptiveRouter, BridgeConfig, ButtonState, EngineAdapter, JsonlReceiptSink,
    LifecyclePolicy, LiveWebEngine, Modifiers, MouseButton, PageSignals, Ray, RouterConfig,
    SpatialInputTranslator, SpatialNodeGeometry, StoragePartitionId, Viewport,
};
use std::error::Error;
use std::sync::Arc;
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("neroa_compatible_adaptive_bridge=debug".parse()?),
        )
        .init();

    let servo_adapter = Arc::new(EngineAdapter::servo_scaffold());

    let chromium_adapter = Arc::new(EngineAdapter::chromium_scaffold());

    let servo: Arc<dyn LiveWebEngine> = servo_adapter.clone();

    let chromium: Arc<dyn LiveWebEngine> = chromium_adapter.clone();

    let router = AdaptiveRouter::new(RouterConfig::default());

    let receipts = Arc::new(JsonlReceiptSink::new("receipts/bridge.jsonl")?);

    let bridge = AdaptiveBridge::new(
        BridgeConfig::default(),
        router,
        servo,
        chromium,
        LifecyclePolicy::default(),
        receipts,
    );

    // Simulate an interactive live webpage.
    let signals = PageSignals::live_html(Url::parse("https://github.com/")?);

    let viewport = Viewport::new(1920, 1080, 1.0);

    let node_id = bridge
        .open_node(
            signals,
            viewport.clone(),
            StoragePartitionId::new("default-profile"),
        )
        .await?;

    let snapshot = bridge.node_snapshot(node_id)?;

    println!(
        "node={} route={:?} engine={:?}",
        snapshot.node_id, snapshot.route, snapshot.engine
    );

    // Spatial plane: two meters wide, 1.125m high, centered
    // at world origin.
    let geometry = SpatialNodeGeometry {
        world_from_local: Mat4::IDENTITY,
        size: Vec2::new(2.0, 1.125),
        viewport,
    };

    // Camera/ray is directly in front of center.
    let ray = Ray::new(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -1.0));

    if let Some(mapped) = SpatialInputTranslator::ray_to_browser(ray, &geometry) {
        println!(
            "ray hit UV=({:.3},{:.3}) device=({:.1},{:.1}) css=({:.1},{:.1})",
            mapped.uv.x,
            mapped.uv.y,
            mapped.device_px.x,
            mapped.device_px.y,
            mapped.css_px.x,
            mapped.css_px.y,
        );
    }

    if let Some(input) = SpatialInputTranslator::pointer_button(
        ray,
        &geometry,
        MouseButton::Left,
        ButtonState::Pressed,
        Modifiers::default(),
    ) {
        bridge.send_input(node_id, input).await?;
    }

    println!("Servo bridge views: {}", servo_adapter.view_count());

    println!("Chromium bridge views: {}", chromium_adapter.view_count());

    Ok(())
}
