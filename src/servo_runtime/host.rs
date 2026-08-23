use std::collections::HashMap;
use std::rc::Rc;

use servo::{
    InputEvent, MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent,
    MouseMoveEvent, RenderingContext, Servo, WebView, WebViewBuilder, WheelDelta, WheelEvent,
    WheelMode,
};
use tokio::sync::mpsc;
use uuid::Uuid;
use webrender_api::units::DevicePoint;

use crate::engine::EngineError;
use crate::types::{
    ActivityState, BrowserInput, ButtonState, MouseButton, PortableWebState, ScrollMode, ViewConfig,
    ViewId,
};

use super::command::ServoCommand;
use super::proxy::ServoEngineProxy;
use super::wake::{ServoHostNotifier, SharedServoHostNotifier};

struct HostView {
    webview: WebView,
    config: ViewConfig,
    portable: PortableWebState,
    activity: ActivityState,
}

/// Event-loop-owned Servo backend.
///
/// This type is intentionally NOT Send/Sync. It owns Servo/WebView/Rc state and
/// must only be driven from the platform event-loop thread. The thread-safe
/// `ServoEngineProxy` communicates with it through `ServoCommand`.
pub struct ServoHost {
    servo: Servo,
    rendering_context: Rc<dyn RenderingContext>,
    rx: mpsc::UnboundedReceiver<ServoCommand>,
    notifier: SharedServoHostNotifier,
    views: HashMap<ViewId, HostView>,
}

impl ServoHost {
    /// Attach the renderer-independent proxy to an event-loop-owned Servo host
    /// without exposing the private command receiver to platform code.
    pub fn attach(
        servo: Servo,
        rendering_context: Rc<dyn RenderingContext>,
        notifier: std::sync::Arc<dyn ServoHostNotifier>,
    ) -> (ServoEngineProxy, Self) {
        let (proxy, rx) = ServoEngineProxy::channel(notifier.clone());
        let host = Self::new(servo, rendering_context, rx, notifier);
        (proxy, host)
    }

    pub(crate) fn new(
        servo: Servo,
        rendering_context: Rc<dyn RenderingContext>,
        rx: mpsc::UnboundedReceiver<ServoCommand>,
        notifier: SharedServoHostNotifier,
    ) -> Self {
        Self {
            servo,
            rendering_context,
            rx,
            notifier,
            views: HashMap::new(),
        }
    }

    /// Drain all pending bridge commands and then give Servo one event-loop turn.
    /// Call this from the platform host whenever its wake event fires.
    pub fn drain_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            self.handle(command);
        }
        self.servo.spin_event_loop();
    }

    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Paint a particular Servo WebView into the host rendering context.
    /// Presentation/blitting remains the responsibility of the platform host.
    pub fn paint(&self, view_id: ViewId) -> Result<(), EngineError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;
        view.webview.paint();
        Ok(())
    }

    fn handle(&mut self, command: ServoCommand) {
        match command {
            ServoCommand::CreateView { config, reply } => {
                let view_id = Uuid::new_v4();
                let portable = PortableWebState::new(config.initial_url.clone());
                let webview = WebViewBuilder::new(&self.servo, self.rendering_context.clone())
                    .url(config.initial_url.clone())
                    .hidpi_scale_factor(euclid::Scale::new(config.viewport.device_scale_factor))
                    .build();
                webview.resize(winit::dpi::PhysicalSize::new(
                    config.viewport.width,
                    config.viewport.height,
                ));

                self.views.insert(
                    view_id,
                    HostView {
                        webview,
                        config,
                        portable,
                        activity: ActivityState::Dormant,
                    },
                );
                let _ = reply.send(Ok(view_id));
                self.notifier.notify();
            }
            ServoCommand::DestroyView { view_id, reply } => {
                let result = if self.views.remove(&view_id).is_some() {
                    Ok(())
                } else {
                    Err(EngineError::ViewNotFound(view_id))
                };
                let _ = reply.send(result);
            }
            ServoCommand::Navigate {
                view_id,
                url,
                reply,
            } => {
                let result = self.with_view_mut(view_id, |view| {
                    view.webview.load(url.clone());
                    let keep = view.portable.history_index.saturating_add(1);
                    view.portable.history.truncate(keep);
                    view.portable.history.push(url.clone());
                    view.portable.history_index = view.portable.history.len().saturating_sub(1);
                    view.portable.url = url;
                    Ok(())
                });
                let _ = reply.send(result);
                self.notifier.notify();
            }
            ServoCommand::Resize {
                view_id,
                viewport,
                reply,
            } => {
                let result = self.with_view_mut(view_id, |view| {
                    view.webview.resize(winit::dpi::PhysicalSize::new(
                        viewport.width,
                        viewport.height,
                    ));
                    view.config.viewport = viewport;
                    Ok(())
                });
                let _ = reply.send(result);
                self.notifier.notify();
            }
            ServoCommand::Input {
                view_id,
                input,
                reply,
            } => {
                let result = self.dispatch_input(view_id, input);
                let _ = reply.send(result);
                self.notifier.notify();
            }
            ServoCommand::SetActivity {
                view_id,
                activity,
                reply,
            } => {
                let result = self.with_view_mut(view_id, |view| {
                    view.activity = activity;
                    match activity {
                        ActivityState::Dormant | ActivityState::Frozen => {
                            view.webview.hide();
                            view.webview.blur();
                        }
                        ActivityState::Throttled { .. } | ActivityState::Active => {
                            view.webview.show();
                        }
                    }
                    Ok(())
                });
                let _ = reply.send(result);
            }
            ServoCommand::ExportState { view_id, reply } => {
                let result = self.views.get(&view_id).map_or_else(
                    || Err(EngineError::ViewNotFound(view_id)),
                    |view| {
                        let mut portable = view.portable.clone();
                        if let Some(url) = view.webview.url() {
                            portable.url = url;
                        }
                        Ok(portable)
                    },
                );
                let _ = reply.send(result);
            }
            ServoCommand::ImportState {
                view_id,
                state,
                reply,
            } => {
                let result = self.with_view_mut(view_id, |view| {
                    view.webview.load(state.url.clone());
                    view.portable = state;
                    Ok(())
                });
                let _ = reply.send(result);
                self.notifier.notify();
            }
            ServoCommand::AcquireFrame { view_id, reply } => {
                let result = if self.views.contains_key(&view_id) {
                    // Deliberately do not call RenderingContext::read_to_image here.
                    // Stock Servo does not expose a stable compositor-shareable texture
                    // handle through this API, so the truthful result is no external GPU
                    // lease until NeroaRenderingContext is implemented.
                    Ok(None)
                } else {
                    Err(EngineError::ViewNotFound(view_id))
                };
                let _ = reply.send(result);
            }
        }
    }

    fn with_view_mut<T>(
        &mut self,
        view_id: ViewId,
        operation: impl FnOnce(&mut HostView) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;
        operation(view)
    }

    fn dispatch_input(&mut self, view_id: ViewId, input: BrowserInput) -> Result<(), EngineError> {
        self.with_view_mut(view_id, |view| {
            match input {
                BrowserInput::PointerMove { position, .. } => {
                    let point = DevicePoint::new(position.x as f32, position.y as f32);
                    view.webview
                        .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
                }
                BrowserInput::PointerButton {
                    position,
                    button,
                    state,
                    ..
                } => {
                    let button = match button {
                        MouseButton::Left => ServoMouseButton::Left,
                        MouseButton::Right => ServoMouseButton::Right,
                        MouseButton::Middle => ServoMouseButton::Middle,
                        MouseButton::Back => ServoMouseButton::Back,
                        MouseButton::Forward => ServoMouseButton::Forward,
                    };
                    let action = match state {
                        ButtonState::Pressed => MouseButtonAction::Down,
                        ButtonState::Released => MouseButtonAction::Up,
                    };
                    let point = DevicePoint::new(position.x as f32, position.y as f32);
                    view.webview.notify_input_event(InputEvent::MouseButton(
                        MouseButtonEvent::new(action, button, point.into()),
                    ));
                }
                BrowserInput::Scroll {
                    position,
                    delta_x,
                    delta_y,
                    mode,
                    ..
                } => {
                    let mode = match mode {
                        ScrollMode::Pixel => WheelMode::DeltaPixel,
                        ScrollMode::Line => WheelMode::DeltaLine,
                    };
                    let point = DevicePoint::new(position.x as f32, position.y as f32);
                    view.webview
                        .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                            WheelDelta {
                                x: delta_x,
                                y: delta_y,
                                z: 0.0,
                                mode,
                            },
                            point.into(),
                        )));
                    view.portable.scroll_x += delta_x;
                    view.portable.scroll_y += delta_y;
                }
                BrowserInput::Focus { focused } => {
                    if focused {
                        view.webview.focus();
                    } else {
                        view.webview.blur();
                    }
                }
                BrowserInput::Key { .. } | BrowserInput::Text { .. } => {
                    return Err(EngineError::Unsupported(
                        "Servo keyboard/text mapping is not wired yet".into(),
                    ));
                }
            }
            Ok(())
        })
    }
}
