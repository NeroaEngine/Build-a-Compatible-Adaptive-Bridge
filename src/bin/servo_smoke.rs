#![cfg(feature = "servo-runtime")]

use std::error::Error;
use std::rc::Rc;
use std::sync::Arc;

use embedder_traits::EventLoopWaker;
use neroa_compatible_adaptive_bridge::{
    ActivityState, LiveWebEngine, ServoHost, ServoHostNotifier, StoragePartitionId, ViewConfig,
    ViewId, Viewport,
};
use servo::{RenderingContext, ServoBuilder, WindowRenderingContext};
use tracing::warn;
use url::Url;
use uuid::Uuid;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

fn main() -> Result<(), Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("failed to create winit event loop");

    let mut app = App::new(&event_loop);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct AppState {
    window: Window,
    rendering_context: Rc<WindowRenderingContext>,
    proxy: neroa_compatible_adaptive_bridge::ServoEngineProxy,
    host: ServoHost,
    view_id: Option<ViewId>,
}

enum App {
    Initial(Waker),
    Running(AppState),
}

impl App {
    fn new(event_loop: &EventLoop<WakerEvent>) -> Self {
        Self::Initial(Waker::new(event_loop.create_proxy()))
    }
}

impl ApplicationHandler<WakerEvent> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Self::Initial(waker) = self else {
            return;
        };

        let display_handle = event_loop
            .display_handle()
            .expect("failed to get display handle");

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Neroa Servo Bridge Smoke")
                    .with_inner_size(winit::dpi::PhysicalSize::new(1000, 700)),
            )
            .expect("failed to create winit window");

        let window_handle = window.window_handle().expect("failed to get window handle");
        let current_size = window.inner_size();
        let initial_size = winit::dpi::PhysicalSize::new(
            current_size.width.max(1),
            current_size.height.max(1),
        );

        let rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, initial_size)
                .expect("failed to create Servo rendering context"),
        );

        rendering_context
            .make_current()
            .expect("failed to make Servo rendering context current");

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        // EventLoopProxy must be created from the original EventLoop. The Waker
        // already owns one, so clone it here rather than asking ActiveEventLoop.
        let host_event_proxy = waker.0.clone();
        let notifier: Arc<dyn ServoHostNotifier> = Arc::new(move || {
            let _ = host_event_proxy.send_event(WakerEvent::Drive);
        });

        let (proxy, host) = ServoHost::attach(servo, rendering_context.clone(), notifier);

        let url = Url::parse(
            "data:text/html,%3Chtml%3E%3Cbody%20style%3D%22margin%3A0%3Bbackground%3A%230b1020%3Bcolor%3Awhite%3Bfont-family%3Asans-serif%3Bdisplay%3Agrid%3Bplace-items%3Acenter%3Bheight%3A100vh%22%3E%3Cdiv%20style%3D%22text-align%3Acenter%22%3E%3Ch1%3ENeroa%20Servo%20Bridge%20Live%3C%2Fh1%3E%3Cp%3ELiveWebEngine%20%E2%86%92%20ServoEngineProxy%20%E2%86%92%20ServoHost%20%E2%86%92%20real%20Servo%200.5.0%20WebView.%3C%2Fp%3E%3C%2Fdiv%3E%3C%2Fbody%3E%3C%2Fhtml%3E",
        )
        .expect("static data URL must parse");

        let config = ViewConfig {
            node_id: Uuid::new_v4(),
            initial_url: url,
            viewport: Viewport::new(
                initial_size.width,
                initial_size.height,
                window.scale_factor() as f32,
            ),
            storage_partition: StoragePartitionId::ephemeral(),
        };

        let request_proxy = proxy.clone();
        let completion_proxy = waker.0.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to create smoke runtime");
            let result = runtime.block_on(async {
                let view_id = request_proxy.create_view(config).await?;
                request_proxy
                    .set_activity(view_id, ActivityState::Active)
                    .await?;
                Ok::<ViewId, neroa_compatible_adaptive_bridge::EngineError>(view_id)
            });
            let result = result.map_err(|error| error.to_string());
            let _ = completion_proxy.send_event(WakerEvent::ViewCreated(result));
        });

        *self = Self::Running(AppState {
            window,
            rendering_context,
            proxy,
            host,
            view_id: None,
        });
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: WakerEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };

        match event {
            WakerEvent::Drive => {
                state.host.drain_commands();
                if state.view_id.is_some() {
                    state.window.request_redraw();
                }
            }
            WakerEvent::ViewCreated(result) => match result {
                Ok(view_id) => {
                    state.view_id = Some(view_id);
                    state.window.set_title("Neroa Servo Bridge Smoke - View Ready");
                    state.host.drain_commands();
                    state.window.request_redraw();
                }
                Err(error) => panic!("Servo bridge create_view failed: {error}"),
            },
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                state.host.drain_commands();
                if let Some(view_id) = state.view_id {
                    state
                        .host
                        .paint(view_id)
                        .expect("failed to paint Servo bridge view");
                    state.rendering_context.present();
                    state.window.set_title("Neroa Servo Bridge Smoke - Presented");
                }
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                // Match Servo 0.5.0's own Winit embedder: resize the WebView,
                // not WindowRenderingContext. The direct Neroa Servo proof uses
                // this same path successfully on Windows.
                if let Some(view_id) = state.view_id {
                    let proxy = state.proxy.clone();
                    let viewport = Viewport::new(
                        size.width,
                        size.height,
                        state.window.scale_factor() as f32,
                    );
                    std::thread::spawn(move || {
                        let runtime = tokio::runtime::Runtime::new()
                            .expect("failed to create resize runtime");
                        let _ = runtime.block_on(proxy.resize(view_id, viewport));
                    });
                }
            }
            _ => {
                state.host.drain_commands();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Self::Running(state) = self {
            // Safety net for platform event-loop implementations: even if a user
            // wake is coalesced, bridge commands still get a deterministic drain
            // before Winit sleeps.
            state.host.drain_commands();
            if state.view_id.is_some() {
                state.window.request_redraw();
            }
        }
    }
}

#[derive(Clone)]
struct Waker(EventLoopProxy<WakerEvent>);

#[derive(Debug)]
enum WakerEvent {
    Drive,
    ViewCreated(Result<ViewId, String>),
}

impl Waker {
    fn new(proxy: EventLoopProxy<WakerEvent>) -> Self {
        Self(proxy)
    }
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if let Err(error) = self.0.send_event(WakerEvent::Drive) {
            warn!(?error, "failed to wake Servo event loop");
        }
    }
}
