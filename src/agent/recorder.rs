//! Training surface.
//!
//! NEROA_TRAINING_SURFACE_V7
//!
//! Records what the agent saw, what it did, and what changed, as JSONL. One
//! line per step, appended and flushed immediately so a crashed or killed run
//! still leaves a usable trace.
//!
//! The shape is deliberately (observation, action, result, next observation):
//! that is what both supervised fine-tuning and offline evaluation want, and
//! it is what a screenshot-based recorder cannot give you cheaply, because
//! every frame would be an image instead of a few kilobytes of structure.
//!
//! Nothing here decides what is worth keeping. It records faithfully and
//! leaves filtering to whoever builds the dataset - a recorder that silently
//! drops failures produces a corpus where the agent never recovers, because
//! it has never seen a mistake.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{ActionOutcome, AgentSurface, PageObservation};
use crate::engine::EngineError;

/// One recorded transition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrainingStep {
    pub session: String,

    pub step: u64,

    /// Monotonic milliseconds since the session started. Wall-clock is left
    /// to the caller so traces stay reproducible.
    pub elapsed_ms: u128,

    /// Verb plus arguments, e.g. `click` / `#submit`.
    pub action: String,

    #[serde(default)]
    pub argument: Option<String>,

    pub outcome: ActionOutcome,

    pub before: PageObservation,

    pub after: PageObservation,

    /// Free-form note from the caller: the goal, the model's reasoning, a
    /// human label. Left to the caller because only they know it.
    #[serde(default)]
    pub note: Option<String>,
}

/// Appends steps to a JSONL file.
pub struct TrainingRecorder {
    path: PathBuf,
    session: String,
    started: std::time::Instant,
    step: u64,
}

impl TrainingRecorder {
    /// Open (or create) a trace file.
    pub fn new(path: impl AsRef<Path>, session: impl Into<String>) -> Result<Self, EngineError> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                EngineError::Internal(format!("training dir: {error}"))
            })?;
        }

        Ok(Self {
            path,
            session: session.into(),
            started: std::time::Instant::now(),
            step: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn steps_recorded(&self) -> u64 {
        self.step
    }

    /// Run an action with the surface, recording the transition around it.
    ///
    /// The observation is taken before and after, so the trace captures what
    /// the action actually changed rather than what it was meant to change.
    pub async fn record<F, Fut>(
        &mut self,
        surface: &AgentSurface,
        action: &str,
        argument: Option<&str>,
        note: Option<&str>,
        act: F,
    ) -> Result<ActionOutcome, EngineError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<ActionOutcome, EngineError>>,
    {
        let before = surface.observe().await?;

        let outcome = act().await?;

        // Let the page settle before observing, or the "after" state is just
        // the "before" state with a different timestamp.
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        let after = surface.observe().await?;

        self.step = self.step.saturating_add(1);

        let entry = TrainingStep {
            session: self.session.clone(),
            step: self.step,
            elapsed_ms: self.started.elapsed().as_millis(),
            action: action.to_string(),
            argument: argument.map(str::to_string),
            outcome: outcome.clone(),
            before,
            after,
            note: note.map(str::to_string),
        };

        self.append(&entry)?;

        Ok(outcome)
    }

    /// Record a step assembled by the caller.
    pub fn append(&self, entry: &TrainingStep) -> Result<(), EngineError> {
        let line = serde_json::to_string(entry)
            .map_err(|error| EngineError::Internal(format!("training encode: {error}")))?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| EngineError::Internal(format!("training open: {error}")))?;

        writeln!(file, "{line}")
            .map_err(|error| EngineError::Internal(format!("training write: {error}")))?;

        // Flushed per step: a killed run should still leave everything it did.
        file.flush()
            .map_err(|error| EngineError::Internal(format!("training flush: {error}")))?;

        Ok(())
    }
}
