#![cfg(feature = "servo-runtime")]

use std::error::Error;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use embedder_traits::EventLoopWaker;
use neroa_compatible_adaptive_bridge::{
    ActivityState, LiveWebEngine, ServoEngineProxy, ServoHost, ServoHostNotifier,
    StoragePartitionId, ViewConfig, ViewId, Viewport,
};
use servo::{RenderingContext, ServoBuilder, SoftwareRenderingContext};
use url::Url;
use uuid::Uuid;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::window::Window;

fn main() -> Result<(), Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("failed to create winit event loop");
    let event_proxy = event_loop.create_proxy();

    let mut app = App::new(event_proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct AppState {
    window: Window,
    proxy: Option<ServoEngineProxy>,
    view_id: Arc<Mutex<Option<ViewId>>>,
}

enum App {
    Initial {
        event_proxy: EventLoopProxy<AppEvent>,
        view_id: Arc<Mutex<Option<ViewId>>>,
    },
    Running(AppState),
}

impl App {
    fn new(event_proxy: EventLoopProxy<AppEvent>) -> Self {
        Self::Initial {
            event_proxy,
            view_id: Arc::new(Mutex::new(None)),
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Self::Initial {
            event_proxy,
            view_id,
        } = self
        else {
            return;
        };

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Neroa Shell - Servo Worker Starting")
                    .with_inner_size(winit::dpi::PhysicalSize::new(1000, 700)),
            )
            .expect("failed to create Neroa shell window");

        spawn_servo_worker(event_proxy.clone(), view_id.clone());

        *self = Self::Running(AppState {
            window,
            proxy: None,
            view_id: view_id.clone(),
        });
    }

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: AppEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };

        match event {
            AppEvent::WorkerReady(proxy) => {
                state.proxy = Some(proxy.clone());
                state.window.set_title("Neroa Shell - Servo Worker Ready");

                let view_id = state.view_id.clone();
                let size = state.window.inner_size();
                let scale_factor = state.window.scale_factor() as f32;
                std::thread::spawn(move || {
                    let runtime =
                        tokio::runtime::Runtime::new().expect("failed to create smoke runtime");
                    let url = Url::parse(
                        "data:text/html,%3Chtml%3E%3Cbody%20style%3D%22margin%3A0%3Bbackground%3A%230b1020%3Bcolor%3Awhite%3Bfont-family%3Asans-serif%3Bdisplay%3Agrid%3Bplace-items%3Acenter%3Bheight%3A100vh%22%3E%3Cdiv%20style%3D%22text-align%3Acenter%22%3E%3Ch1%3ENeroa%20Servo%20Worker%20Live%3C%2Fh1%3E%3Cp%3EServo%20is%20running%20off%20the%20visible%20Neroa%20window%20thread.%3C%2Fp%3E%3C%2Fdiv%3E%3C%2Fbody%3E%3C%2Fhtml%3E",
                    )
                    .expect("static data URL must parse");
                    let config = ViewConfig {
                        node_id: Uuid::new_v4(),
                        initial_url: url,
                        viewport: Viewport::new(size.width.max(1), size.height.max(1), scale_factor),
                        storage_partition: StoragePartitionId::ephemeral(),
                    };

                    let result = runtime.block_on(async {
                        let id = proxy.create_view(config).await?;
                        *view_id.lock().expect("view id lock poisoned") = Some(id);
                        proxy.set_activity(id, ActivityState::Active).await?;
                        Ok::<ViewId, neroa_compatible_adaptive_bridge::EngineError>(id)
                    });

                    if let Err(error) = result {
                        eprintln!("Servo worker create_view failed: {error}");
                    }
                });
            }
            AppEvent::WorkerStatus(status) => {
                state.window.set_title(&format!("Neroa Shell - {status}"));
            }
            AppEvent::WorkerFailed(error) => {
                state.window.set_title("Neroa Shell - Servo Worker Failed");
                eprintln!("Servo worker failed: {error}");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(_state) = self else {
            return;
        };

        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }
}

fn spawn_servo_worker(
    shell_events: EventLoopProxy<AppEvent>,
    shared_view_id: Arc<Mutex<Option<ViewId>>>,
) {
    std::thread::Builder::new()
        .name("neroa-servo-host".into())
        .spawn(move || {
            let (wake_tx, wake_rx) = mpsc::channel::<()>();
            let servo_waker = ThreadWaker(wake_tx.clone());

            let rendering_context = match SoftwareRenderingContext::new(
                winit::dpi::PhysicalSize::new(1000, 700),
            ) {
                Ok(context) => Rc::new(context),
                Err(error) => {
                    let _ = shell_events.send_event(AppEvent::WorkerFailed(format!(
                        "software rendering context: {error:?}"
                    )));
                    return;
                }
            };

            if let Err(error) = rendering_context.make_current() {
                let _ = shell_events.send_event(AppEvent::WorkerFailed(format!(
                    "make current: {error:?}"
                )));
                return;
            }

            let servo = ServoBuilder::default()
                .event_loop_waker(Box::new(servo_waker.clone()))
                .build();
            servo.setup_logging();

            let notifier_tx = wake_tx.clone();
            let notifier: Arc<dyn ServoHostNotifier> = Arc::new(move || {
                let _ = notifier_tx.send(());
            });
            let (proxy, mut host) = ServoHost::attach(servo, rendering_context.clone(), notifier);

            if shell_events
                .send_event(AppEvent::WorkerReady(proxy))
                .is_err()
            {
                return;
            }

            loop {
                match wake_rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
                        host.drain_commands();

                        let current_view = *shared_view_id
                            .lock()
                            .expect("shared view id lock poisoned");
                        let Some(view_id) = current_view else {
                            continue;
                        };

                        let frame_ready = host.take_frame_ready(view_id).unwrap_or(false);
                        if frame_ready {
                            if let Err(error) = host.paint(view_id) {
                                let _ = shell_events.send_event(AppEvent::WorkerFailed(format!(
                                    "paint: {error}"
                                )));
                                return;
                            }
                            rendering_context.present();
                        }

                        match host.diagnostic_summary(view_id) {
                            Ok(summary) => {
                                if shell_events
                                    .send_event(AppEvent::WorkerStatus(summary))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ = shell_events.send_event(AppEvent::WorkerFailed(format!(
                                    "diagnostics: {error}"
                                )));
                                return;
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("failed to spawn Servo host thread");
}

#[derive(Clone)]
struct ThreadWaker(mpsc::Sender<()>);

impl EventLoopWaker for ThreadWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.0.send(());
    }
}

enum AppEvent {
    WorkerReady(ServoEngineProxy),
    WorkerStatus(String),
    WorkerFailed(String),
}
