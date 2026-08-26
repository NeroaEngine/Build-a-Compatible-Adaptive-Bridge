use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use servo::{
    InputEvent, InputEventId, InputEventResult, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, RenderingContext, Servo, WebView, WebViewBuilder,
    WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
use tokio::sync::mpsc;
use uuid::Uuid;
use webrender_api::units::DevicePoint;

use crate::engine::EngineError;
use crate::types::{
    ActivityState, BrowserInput, ButtonState, MouseButton, PortableWebState, ScrollMode,
    ViewConfig, ViewId,
};

use super::command::ServoCommand;
use super::frame::{NoSharedGpuFrameSource, ServoFrameSource};
use super::input::{committed_text_input, keyboard_input};
use super::proxy::ServoEngineProxy;
use super::wake::{ServoHostNotifier, SharedServoHostNotifier};

struct HostView {
    traced_input_ids: TracedInputIds,
    webview: WebView,
    config: ViewConfig,
    portable: PortableWebState,
    activity: ActivityState,
    frame_ready_count: Rc<Cell<u64>>,
    frame_ready_pending: Rc<Cell<bool>>,
    // NEROA_NAVIGATION_FRAME_QUARANTINE_V1B
    navigation_generation: Rc<Cell<u64>>,
    navigation_loading: Rc<Cell<bool>>,
    navigation_frame_seen: Rc<Cell<bool>>,
}

type TracedInputIds = Rc<std::cell::RefCell<std::collections::HashSet<InputEventId>>>;

struct HostWebViewDelegate {
    traced_input_ids: TracedInputIds,
    notifier: SharedServoHostNotifier,
    frame_ready_count: Rc<Cell<u64>>,
    frame_ready_pending: Rc<Cell<bool>>,
    navigation_generation: Rc<Cell<u64>>,
    navigation_loading: Rc<Cell<bool>>,
    navigation_frame_seen: Rc<Cell<bool>>,
}

impl WebViewDelegate for HostWebViewDelegate {
    // NEROA_NAVIGATION_WAKE_COALESCE_V1C
    fn notify_load_status_changed(&self, _webview: WebView, status: servo::LoadStatus) {
        if status != servo::LoadStatus::Complete {
            return;
        }

        if !self.navigation_loading.replace(false) {
            return;
        }

        let generation = self.navigation_generation.get();

        eprintln!("NEROA_NAV_ADAPTER_LOAD_COMPLETE generation={}", generation,);

        // NEROA_NAV_QUARANTINE_RELEASE_V2H
        //
        // Always hand the compositor back to the normal path. Gating the
        // release on having seen a quarantined frame meant a document that
        // finished loading before its first frame left frame_ready_pending
        // false with nothing scheduled to set it.
        self.navigation_frame_seen.set(false);

        if !self.frame_ready_pending.replace(true) {
            eprintln!("NEROA_NAV_ADAPTER_FRAME_RELEASE generation={}", generation,);

            self.notifier.notify();
        }
    }

    // NEROA_INPUT_OUTCOME_TRACE_V2H
    //
    // Servo silently discards any pointer event whose hit test is empty
    // (paint::webview_renderer: "Empty hit test result ... ignoring").
    // Pair this with NEROA_INPUT_DISPATCH to see, per event id, whether a
    // click actually reached the DOM or was dropped on the floor.
    fn notify_input_event_handled(
        &self,
        _webview: WebView,
        event_id: InputEventId,
        result: InputEventResult,
    ) {
        if !self.traced_input_ids.borrow_mut().remove(&event_id) {
            return;
        }

        eprintln!(
            "NEROA_INPUT_RESULT id={:?} consumed={} default_prevented={} dispatch_failed={}",
            event_id,
            result.contains(InputEventResult::Consumed),
            result.contains(InputEventResult::DefaultPrevented),
            result.contains(InputEventResult::DispatchFailed),
        );
    }

    // NEROA_WEBVIEW_FAILURE_VISIBILITY_V2H
    //
    // Without these, a dead content process is indistinguishable from a
    // slow page: both produce no log output at all.
    fn notify_crashed(&self, _webview: WebView, reason: String, _backtrace: Option<String>) {
        eprintln!("NEROA_WEBVIEW_CRASHED reason={}", reason);

        self.navigation_loading.set(false);

        if !self.frame_ready_pending.replace(true) {
            self.notifier.notify();
        }
    }

    fn show_console_message(&self, _webview: WebView, level: servo::ConsoleLogLevel, message: String) {
        if matches!(level, servo::ConsoleLogLevel::Error) {
            eprintln!("NEROA_WEBVIEW_CONSOLE_ERROR {}", message);
        }
    }

    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.frame_ready_count
            .set(self.frame_ready_count.get().saturating_add(1));

        // During navigation we retain only the fact that at least
        // one new-document frame exists. Do not wake the host for
        // every intermediate Servo frame.
        if self.navigation_loading.get() {
            if !self.navigation_frame_seen.replace(true) {
                eprintln!(
                    "NEROA_NAV_ADAPTER_FIRST_FRAME_QUARANTINED generation={}",
                    self.navigation_generation.get(),
                );
            }

            return;
        }

        // Level-triggered frame readiness: at most one outstanding
        // wake exists until the host consumes frame_ready_pending.
        if !self.frame_ready_pending.replace(true) {
            self.notifier.notify();
        }
    }
}

/// Servo backend owned by the dedicated Servo host thread.
///
/// This type is intentionally NOT Send/Sync. It owns Servo/WebView/Rc state and
/// must only be driven from its owner thread. The thread-safe `ServoEngineProxy`
/// communicates with it through `ServoCommand`.
pub struct ServoHost {
    servo: Servo,
    rendering_context: Rc<dyn RenderingContext>,
    frame_source: Rc<dyn ServoFrameSource>,
    rx: mpsc::UnboundedReceiver<ServoCommand>,
    notifier: SharedServoHostNotifier,
    views: HashMap<ViewId, HostView>,
}

impl ServoHost {
    /// Attach the renderer-independent proxy with the fail-closed default frame
    /// source. Until a Neroa GPU exporter is installed, `acquire_frame()` returns
    /// `None` and never performs CPU pixel readback.
    pub fn attach(
        servo: Servo,
        rendering_context: Rc<dyn RenderingContext>,
        notifier: std::sync::Arc<dyn ServoHostNotifier>,
    ) -> (ServoEngineProxy, Self) {
        Self::attach_with_frame_source(
            servo,
            rendering_context,
            notifier,
            Rc::new(NoSharedGpuFrameSource),
        )
    }

    /// Attach a concrete GPU frame exporter owned by the Servo host thread.
    ///
    /// The frame source may expose only compositor-shareable GPU resources. It
    /// must not implement this seam using CPU RGBA readback.
    pub fn attach_with_frame_source(
        servo: Servo,
        rendering_context: Rc<dyn RenderingContext>,
        notifier: std::sync::Arc<dyn ServoHostNotifier>,
        frame_source: Rc<dyn ServoFrameSource>,
    ) -> (ServoEngineProxy, Self) {
        let external_gpu_surface = frame_source.supports_external_gpu_surface();
        let (proxy, rx) = ServoEngineProxy::channel(notifier.clone(), external_gpu_surface);
        let host = Self::new(servo, rendering_context, frame_source, rx, notifier);
        (proxy, host)
    }

    pub(crate) fn new(
        servo: Servo,
        rendering_context: Rc<dyn RenderingContext>,
        frame_source: Rc<dyn ServoFrameSource>,
        rx: mpsc::UnboundedReceiver<ServoCommand>,
        notifier: SharedServoHostNotifier,
    ) -> Self {
        Self {
            servo,
            rendering_context,
            frame_source,
            rx,
            notifier,
            views: HashMap::new(),
        }
    }

    /// Drain pending bridge commands and give Servo one event-loop turn.
    /// A wake is not equivalent to a frame-ready notification.
    pub fn drain_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            self.handle(command);
        }
        self.servo.spin_event_loop();
    }

    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Consume the current frame-ready signal for a view.
    ///
    /// This lets the platform event loop distinguish a generic Servo/command
    /// wake from a real compositor frame notification, preventing redraw loops.
    pub fn take_frame_ready(&self, view_id: ViewId) -> Result<bool, EngineError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;
        Ok(view.frame_ready_pending.replace(false))
    }

    /// Human-readable, non-pixel diagnostic state for the transitional smoke
    /// host. This intentionally does not read the rendering surface back to CPU.
    pub fn diagnostic_summary(&self, view_id: ViewId) -> Result<String, EngineError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(EngineError::ViewNotFound(view_id))?;
        Ok(format!(
            "load={:?} frames={} activity={:?}",
            view.webview.load_status(),
            view.frame_ready_count.get(),
            view.activity,
        ))
    }

    /// Paint a particular Servo WebView into the host rendering context.
    /// Presentation/blitting remains the responsibility of the host integration.
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
                let frame_ready_count = Rc::new(Cell::new(0));
                let frame_ready_pending = Rc::new(Cell::new(false));
                let navigation_generation = Rc::new(Cell::new(0));
                let navigation_loading = Rc::new(Cell::new(false));
                let navigation_frame_seen = Rc::new(Cell::new(false));
                let traced_input_ids: TracedInputIds = Rc::new(std::cell::RefCell::new(
                    std::collections::HashSet::new(),
                ));
                let delegate = Rc::new(HostWebViewDelegate {
                    traced_input_ids: traced_input_ids.clone(),
                    notifier: self.notifier.clone(),
                    frame_ready_count: frame_ready_count.clone(),
                    frame_ready_pending: frame_ready_pending.clone(),
                    navigation_generation: navigation_generation.clone(),
                    navigation_loading: navigation_loading.clone(),
                    navigation_frame_seen: navigation_frame_seen.clone(),
                });
                let webview = WebViewBuilder::new(&self.servo, self.rendering_context.clone())
                    .url(config.initial_url.clone())
                    .hidpi_scale_factor(euclid::Scale::new(config.viewport.device_scale_factor))
                    .delegate(delegate)
                    .build();
                webview.resize(winit::dpi::PhysicalSize::new(
                    config.viewport.width,
                    config.viewport.height,
                ));

                self.views.insert(
                    view_id,
                    HostView {
                        traced_input_ids,
                        webview,
                        config,
                        portable,
                        activity: ActivityState::Dormant,
                        frame_ready_count,
                        frame_ready_pending,
                        navigation_generation,
                        navigation_loading,
                        navigation_frame_seen,
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
                    let generation = view.navigation_generation.get().saturating_add(1);

                    // NEROA_NAV_QUARANTINE_RETRY_V2H
                    //
                    // Re-arming the quarantine for a navigation already in
                    // flight discarded the pending exit and cleared
                    // frame_ready_pending, so repeatedly pressing Go on a
                    // slow page starved the very frame it was waiting for.
                    // Retries now keep the in-flight quarantine state.
                    let already_loading = view.navigation_loading.get();

                    view.navigation_generation.set(generation);
                    view.navigation_loading.set(true);

                    if !already_loading {
                        view.navigation_frame_seen.set(false);
                        view.frame_ready_pending.set(false);
                    }

                    eprintln!(
                        "NEROA_NAV_ADAPTER_BEGIN generation={} url={} retry={}",
                        generation, url, already_loading,
                    );

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
                // NEROA_INPUT_WAKE_COALESCE_V2G
                //
                // proxy.send() already woke the host, and drain_commands()
                // spins Servo's event loop immediately after this returns.
                // Notifying again only schedules a second wake that drains
                // an empty queue - two event-loop spins per pointer event.
                let result = self.dispatch_input(view_id, input);
                let _ = reply.send(result);
            }
            ServoCommand::SetActivity {
                view_id,
                activity,
                reply,
            } => {
                let result = self.with_view_mut(view_id, |view| {
                    // Activity is Neroa scheduling policy. Do not equate it with
                    // Servo visibility until a distinct visibility contract exists.
                    view.activity = activity;
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
                let result = self.views.get(&view_id).map_or_else(
                    || Err(EngineError::ViewNotFound(view_id)),
                    |view| {
                        let generation = view.frame_ready_count.get();
                        if generation == 0 {
                            return Ok(None);
                        }

                        self.frame_source.acquire_surface(
                            view_id,
                            &view.config.viewport,
                            generation,
                        )
                    },
                );
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
                        .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                            point.into(),
                        )));
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
                    let event_id = view.webview.notify_input_event(InputEvent::MouseButton(
                        MouseButtonEvent::new(action, button, point.into()),
                    ));

                    // NEROA_INPUT_OUTCOME_TRACE_V2H
                    eprintln!(
                        "NEROA_INPUT_DISPATCH id={:?} kind=button x={:.1} y={:.1}",
                        event_id, position.x, position.y,
                    );

                    view.traced_input_ids.borrow_mut().insert(event_id);
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
                BrowserInput::Key {
                    physical_code,
                    logical_key,
                    state,
                    modifiers,
                } => {
                    view.webview.notify_input_event(keyboard_input(
                        &physical_code,
                        &logical_key,
                        state,
                        modifiers,
                    ));
                }
                BrowserInput::Text { text } => {
                    view.webview.notify_input_event(committed_text_input(text));
                }
            }
            Ok(())
        })
    }
}
