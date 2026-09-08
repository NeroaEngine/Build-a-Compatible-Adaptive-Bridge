#![cfg(all(feature = "servo-runtime", feature = "headless-http"))]

use std::{
    env,
    error::Error,
    net::SocketAddr,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

use embedder_traits::EventLoopWaker;
use neroa_compatible_adaptive_bridge::{
    ServoHost, ServoHostNotifier,
    headless_agent_http::{HeadlessAgentHttpState, router},
    headless_session::HeadlessSessionManager,
};
use servo::{RenderingContext, ServoBuilder, SoftwareRenderingContext};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| "failed to install rustls crypto provider")?;

    let bind: SocketAddr = env::var("NEROA_HEADLESS_BIND")
        .unwrap_or_else(|_| "127.0.0.1:19091".to_owned())
        .parse()?;
    let service_token = optional_secret("NEROA_HEADLESS_SERVICE_TOKEN_FILE")?;
    let bound_profile_id = env::var("NEROA_HEADLESS_PROFILE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let bootstrap_width = parse_or("NEROA_HEADLESS_BOOTSTRAP_WIDTH", 1920u32)?;
    let bootstrap_height = parse_or("NEROA_HEADLESS_BOOTSTRAP_HEIGHT", 1080u32)?;

    let proxy = spawn_servo_host(bootstrap_width, bootstrap_height)?;
    let sessions = HeadlessSessionManager::new(proxy);
    let state = HeadlessAgentHttpState::new(sessions, service_token)
        .with_bound_profile(bound_profile_id.clone());
    let app = router(state);

    let listener = TcpListener::bind(bind).await?;

    // Servo owns the process-wide `log` logger after `servo.setup_logging()`.
    // `tracing_subscriber::fmt().init()` also tries to install a LogTracer,
    // which panics with SetLoggerError when Servo has already claimed it.
    // Install only the tracing subscriber here; leave the `log` facade to Servo.
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "neroa_compatible_adaptive_bridge=info".into()),
        )
        .json()
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| format!("failed to install tracing subscriber: {error}"))?;

    tracing::info!(
        %bind,
        profile_id = bound_profile_id.as_deref().unwrap_or("unbound-development"),
        "Neroa Spatial Browser headless server listening"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn spawn_servo_host(
    width: u32,
    height: u32,
) -> Result<neroa_compatible_adaptive_bridge::ServoEngineProxy, Box<dyn Error>> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);

    std::thread::Builder::new()
        .name("neroa-headless-servo-host".into())
        .spawn(move || {
            let (wake_tx, wake_rx) = mpsc::channel::<()>();
            let waker = ThreadWaker(wake_tx.clone());

            let rendering_context = match SoftwareRenderingContext::new(
                winit::dpi::PhysicalSize::new(width.max(1), height.max(1)),
            ) {
                Ok(context) => Rc::new(context),
                Err(error) => {
                    let _ = ready_tx.send(Err(format!(
                        "software rendering context creation failed: {error:?}"
                    )));
                    return;
                }
            };

            if let Err(error) = rendering_context.make_current() {
                let _ = ready_tx.send(Err(format!(
                    "software rendering context make_current failed: {error:?}"
                )));
                return;
            }

            let servo = ServoBuilder::default()
                .event_loop_waker(Box::new(waker))
                .build();
            servo.setup_logging();

            let notifier_tx = wake_tx.clone();
            let notifier: Arc<dyn ServoHostNotifier> = Arc::new(move || {
                let _ = notifier_tx.send(());
            });

            let (proxy, mut host) = ServoHost::attach(servo, rendering_context, notifier);
            if ready_tx.send(Ok(proxy)).is_err() {
                return;
            }

            loop {
                match wake_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
                        host.drain_commands();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })?;

    match ready_rx.recv_timeout(Duration::from_secs(15))? {
        Ok(proxy) => Ok(proxy),
        Err(message) => Err(message.into()),
    }
}

fn optional_secret(name: &str) -> Result<Option<String>, Box<dyn Error>> {
    let Some(path) = env::var(name).ok().filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let value = std::fs::read_to_string(&path)?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("secret file {path} is empty").into());
    }
    Ok(Some(value.to_owned()))
}

fn parse_or<T>(name: &str, default: T) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr + Copy,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(value) => value
            .parse::<T>()
            .map_err(|error| format!("invalid {name}: {error}").into()),
        Err(_) => Ok(default),
    }
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
