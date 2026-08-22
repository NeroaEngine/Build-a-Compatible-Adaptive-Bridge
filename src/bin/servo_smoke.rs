#![cfg(feature = "servo-runtime")]

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use embedder_traits::EventLoopWaker;
use euclid::{Scale, Size2D};
use servo::{
    RenderingContext, Servo, ServoBuilder, WebView, WebViewBuilder, WindowRenderingContext,
};
use tracing::warn;
use url::Url;
use webrender_api::units::DevicePixel;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
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
    servo: Servo,
    rendering_context: Rc<WindowRenderingContext>,
    webviews: RefCell<Vec<WebView>>,
}

impl servo::WebViewDelegate for AppState {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }
}

enum App {
    Initial(Waker),
    Running(Rc<AppState>),
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
            .create_window(Window::default_attributes().with_title("Neroa Servo Smoke"))
            .expect("failed to create winit window");

        let window_handle = window.window_handle().expect("failed to get window handle");

        let rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("failed to create Servo rendering context"),
        );

        rendering_context
            .make_current()
            .expect("failed to make Servo rendering context current");

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        let app_state = Rc::new(AppState {
            window,
            servo,
            rendering_context,
            webviews: RefCell::new(Vec::new()),
        });

        let url = Url::parse("https://example.com/").expect("static URL must parse");

        let webview = WebViewBuilder::new(&app_state.servo, app_state.rendering_context.clone())
            .url(url)
            .hidpi_scale_factor(Scale::new(app_state.window.scale_factor() as f32))
            .delegate(app_state.clone())
            .build();

        app_state.webviews.borrow_mut().push(webview);
        *self = Self::Running(app_state);
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _event: WakerEvent,
    ) {
        if let Self::Running(state) = self {
            state.servo.spin_event_loop();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Self::Running(state) = self {
            state.servo.spin_event_loop();
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Self::Running(state) = self {
                    if let Some(webview) = state.webviews.borrow().last() {
                        webview.paint();
                        state.rendering_context.present();
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Self::Running(state) = self {
                    state.rendering_context.resize(size);
                    if let Some(webview) = state.webviews.borrow().last() {
                        webview.resize(size);
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Waker(EventLoopProxy<WakerEvent>);

#[derive(Debug)]
struct WakerEvent;

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
        if let Err(error) = self.0.send_event(WakerEvent) {
            warn!(?error, "failed to wake Servo event loop");
        }
    }
}

#[allow(dead_code)]
fn winit_size_to_euclid_size<T>(size: PhysicalSize<T>) -> Size2D<T, DevicePixel> {
    Size2D::new(size.width, size.height)
}
