//! Agent control surface.
//!
//! NEROA_AGENT_SURFACE_V7
//!
//! The verbs an automated caller needs to drive a page: observe it, act on
//! it, wait for it to settle. Every one is built on script evaluation and the
//! input path that already exist, so there is no second engine contract and
//! nothing to keep in sync.
//!
//! What makes this different from driving Chrome over CDP is not the API - it
//! is deliberately familiar - but that it runs in-process against the same
//! engine the compositor is drawing. There is no socket, no JSON-RPC round
//! trip, and no screenshot needed to know what is on the page.

mod observation;
mod recorder;

pub use observation::{ElementHandle, PageObservation};
pub use recorder::{TrainingRecorder, TrainingStep};

use serde::{Deserialize, Serialize};

use crate::engine::EngineError;
use crate::servo_runtime::ServoEngineProxy;
use crate::types::ViewId;

/// Outcome of a single agent action.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub ok: bool,

    /// Why the action failed, when it did. Agents recover far better from a
    /// reason than from a bare false.
    pub detail: Option<String>,
}

impl ActionOutcome {
    fn ok() -> Self {
        Self { ok: true, detail: None }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self { ok: false, detail: Some(detail.into()) }
    }
}

/// Drives one view.
#[derive(Clone)]
pub struct AgentSurface {
    proxy: ServoEngineProxy,
    view_id: ViewId,
}

impl AgentSurface {
    pub fn new(proxy: ServoEngineProxy, view_id: ViewId) -> Self {
        Self { proxy, view_id }
    }

    pub fn view_id(&self) -> ViewId {
        self.view_id
    }

    /// Evaluate a script and decode the JSON result.
    pub async fn eval<T: for<'de> Deserialize<'de>>(
        &self,
        script: impl Into<String>,
    ) -> Result<T, EngineError> {
        let raw = self.proxy.evaluate(self.view_id, script.into()).await?;

        serde_json::from_str(&raw)
            .map_err(|error| EngineError::Internal(format!("agent decode failed: {error}: {raw}")))
    }

    /// A structured view of the page: identity, readable text, and the
    /// elements worth acting on.
    ///
    /// This is the observation an agent reasons over. It is deliberately text,
    /// not pixels: no screenshot tokens, no OCR, and selectors that can be
    /// acted on directly rather than coordinates that have to be guessed.
    pub async fn observe(&self) -> Result<PageObservation, EngineError> {
        self.eval(observation::OBSERVE_SCRIPT).await
    }

    /// Visible text of the page, collapsed.
    pub async fn read_text(&self) -> Result<String, EngineError> {
        self.eval(observation::READ_TEXT_SCRIPT).await
    }

    pub async fn click(&self, selector: &str) -> Result<ActionOutcome, EngineError> {
        self.act("click", selector, observation::CLICK_BODY).await
    }

    /// Set a field value and fire the events frameworks listen for.
    ///
    /// Assigning `.value` alone is invisible to React and friends, which is a
    /// common reason naive automation appears to type but changes nothing.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<ActionOutcome, EngineError> {
        let encoded = encode(text)?;

        let body = observation::TYPE_BODY.replace("__VALUE__", &encoded);

        self.act("type", selector, &body).await
    }

    /// Submit the form owning `selector`, or the element itself if it is one.
    pub async fn submit(&self, selector: &str) -> Result<ActionOutcome, EngineError> {
        self.act("submit", selector, observation::SUBMIT_BODY).await
    }

    /// Poll until `selector` exists, or the budget expires.
    ///
    /// Returns false on timeout rather than erroring: not-yet-present is a
    /// normal outcome an agent should be able to branch on.
    pub async fn wait_for(
        &self,
        selector: &str,
        timeout: std::time::Duration,
    ) -> Result<bool, EngineError> {
        let deadline = std::time::Instant::now() + timeout;

        let script = format!("(() => !!document.querySelector({}))()", encode(selector)?);

        loop {
            if self.eval::<bool>(script.clone()).await.unwrap_or(false) {
                return Ok(true);
            }

            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }

            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
    }

    /// Wait until the document has finished loading.
    pub async fn wait_for_load(&self, timeout: std::time::Duration) -> Result<bool, EngineError> {
        let deadline = std::time::Instant::now() + timeout;

        loop {
            let ready: String = self
                .eval("(() => document.readyState)()")
                .await
                .unwrap_or_else(|_| "loading".to_string());

            if ready == "complete" {
                return Ok(true);
            }

            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Pull structured records out of the page.
    ///
    /// `fields` maps an output name to a CSS selector evaluated relative to
    /// each match of `selector`. This is how an agent turns a listing into
    /// rows without a model reading every pixel.
    pub async fn extract(
        &self,
        selector: &str,
        fields: &[(&str, &str)],
    ) -> Result<Vec<serde_json::Value>, EngineError> {
        let map: std::collections::BTreeMap<String, String> = fields
            .iter()
            .map(|(name, sel)| ((*name).to_string(), (*sel).to_string()))
            .collect();

        let spec = serde_json::to_string(&map)
            .map_err(|error| EngineError::Internal(format!("encode failed: {error}")))?;

        let script = observation::EXTRACT_SCRIPT
            .replace("__SPEC__", &spec)
            .replace("__ROOT__", &encode(selector)?);

        self.eval(script).await
    }

    pub async fn navigate(&self, url: url::Url) -> Result<(), EngineError> {
        self.proxy.queue_navigation(self.view_id, url)
    }

    /// Navigate and wait for the new document, not the old one.
    ///
    /// NEROA_AGENT_NAVIGATION_RACE_V7: navigation is queued asynchronously,
    /// so checking readyState straight after navigate() observes the document
    /// being replaced - which is already "complete" - and returns immediately
    /// with the previous page still loaded.
    ///
    /// Establishing the starting URL first matters just as much: if that
    /// probe fails and is treated as empty, every subsequent URL differs from
    /// it and the wait falls through on the first poll. The starting URL is
    /// therefore confirmed before navigating, and the destination is matched
    /// on origin+path so a redirect or an added query string still counts as
    /// arrival.
    pub async fn navigate_and_wait(
        &self,
        url: url::Url,
        timeout: std::time::Duration,
    ) -> Result<bool, EngineError> {
        let deadline = std::time::Instant::now() + timeout;

        let mut before = String::new();

        while before.is_empty() && std::time::Instant::now() < deadline {
            before = self
                .eval::<String>("(() => location.href)()")
                .await
                .unwrap_or_default();

            if before.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            }
        }

        if before.is_empty() {
            return Err(EngineError::Internal(
                "agent: view never reported a location before navigating".to_string(),
            ));
        }

        let want = format!("{}{}", url.origin().ascii_serialization(), url.path());

        self.navigate(url).await?;

        loop {
            let current = self
                .eval::<String>("(() => location.href)()")
                .await
                .unwrap_or_default();

            let arrived = !current.is_empty()
                && current != before
                && (current.starts_with(&want) || !current.starts_with("data:"));

            if arrived {
                break;
            }

            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }

            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());

        self.wait_for_load(remaining).await
    }

    pub async fn back(&self) -> Result<bool, EngineError> {
        self.proxy.traverse(self.view_id, -1).await
    }

    pub async fn forward(&self) -> Result<bool, EngineError> {
        self.proxy.traverse(self.view_id, 1).await
    }

    pub async fn reload(&self) -> Result<(), EngineError> {
        self.proxy.reload(self.view_id).await
    }

    async fn act(
        &self,
        verb: &str,
        selector: &str,
        body: &str,
    ) -> Result<ActionOutcome, EngineError> {
        let script = observation::ACT_SCRIPT
            .replace("__SELECTOR__", &encode(selector)?)
            .replace("__BODY__", body);

        let found: bool = self.eval(script).await?;

        Ok(if found {
            ActionOutcome::ok()
        } else {
            ActionOutcome::failed(format!("{verb}: no element matched {selector}"))
        })
    }
}

fn encode(value: &str) -> Result<String, EngineError> {
    serde_json::to_string(value)
        .map_err(|error| EngineError::Internal(format!("encode failed: {error}")))
}
