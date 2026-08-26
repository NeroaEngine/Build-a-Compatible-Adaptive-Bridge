//! Forge-shaped training receipts.
//!
//! NEROA_TRAINING_RECEIPTS_V13
//!
//! The Forge training room ingests receipts, then runs them through its Merkle
//! deck, Avalanche attestation, lineage, and finally shards them into training.
//! Those stages live in Forge; this module produces the receipt they consume,
//! in Forge's envelope, so a browsing session drops straight into that
//! pipeline instead of being a loose JSONL file no one can attest.
//!
//! Two properties the downstream stages depend on:
//!
//!  * content_hash - sha256 over the canonical payload, so the Merkle deck can
//!    build a tree whose leaves are exactly these receipts and nothing can be
//!    altered without changing the root.
//!  * parent_ref - each receipt points at the previous one in the session,
//!    forming the hash-linked lineage the pipeline walks. A gap or a rewrite
//!    breaks the chain visibly rather than silently.
//!
//! The receipt_id is derived from the content hash, so it is deterministic:
//! the same step produces the same id, which is what lets Forge dedupe and
//! attest without a coordinator assigning ids.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::TrainingStep;

/// Refs that place a receipt in the Forge tenancy tree. Supplied once per
/// session; Forge rejects a receipt that is not anchored to a tenant.
#[derive(Clone, Debug)]
pub struct ReceiptContext {
    pub tenant_ref: String,
    pub workspace_ref: String,
    pub runtime_ref: String,
    pub actor_ref: String,
    pub session_ref: String,
}

impl Default for ReceiptContext {
    fn default() -> Self {
        Self {
            tenant_ref: "tenant:neroa".to_string(),
            workspace_ref: "workspace:spatial-browser".to_string(),
            runtime_ref: "neroa-spatial-browser".to_string(),
            actor_ref: "actor:agent".to_string(),
            session_ref: "session:unknown".to_string(),
        }
    }
}

/// One training step in Forge's receipt envelope.
///
/// Field names mirror the Forge receipt fixtures (receipt_id, receipt_type,
/// tenant_ref, workspace_ref, runtime_ref, created_at) so validation there
/// accepts it without a translation layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrainingReceipt {
    pub receipt_id: String,
    pub receipt_type: String,

    pub tenant_ref: String,
    pub workspace_ref: String,
    pub runtime_ref: String,
    pub actor_ref: String,
    pub session_ref: String,

    /// sha256 of the canonical payload. The Merkle leaf.
    pub content_hash: String,

    /// The previous receipt in this session, or null for the first. The
    /// lineage link.
    pub parent_ref: Option<String>,

    /// Position in the session, starting at 1.
    pub sequence: u64,

    pub created_at: String,

    /// The training payload itself: observation, action, outcome.
    pub payload: TrainingStep,
}

/// Turns training steps into a hash-linked chain of Forge receipts.
///
/// Not thread-safe by design: a lineage chain is inherently ordered, so the
/// caller serialises steps through one emitter.
pub struct ReceiptEmitter {
    context: ReceiptContext,
    parent: Option<String>,
    sequence: u64,
}

impl ReceiptEmitter {
    pub fn new(context: ReceiptContext) -> Self {
        Self {
            context,
            parent: None,
            sequence: 0,
        }
    }

    /// Wrap a step, linking it to the previous receipt.
    ///
    /// `now_unix_ms` is passed in rather than read here so the emitter has no
    /// hidden clock dependency and a replay produces identical receipts.
    pub fn emit(&mut self, step: &TrainingStep, now_unix_ms: u64) -> TrainingReceipt {
        self.sequence += 1;

        let content_hash = content_hash(step);

        // content_hash stays pure - identical steps share it, which is what
        // lets the Merkle deck dedupe and attest. The receipt_id folds in the
        // sequence so the lineage chain is strictly ordered even when two
        // steps have identical content (same page, same action); otherwise a
        // repeated step would point at its own id and break the chain.
        let receipt_id = format!("ai:training:{}:{}", self.sequence, &content_hash[..24]);

        let receipt = TrainingReceipt {
            receipt_id: receipt_id.clone(),
            receipt_type: "ai_training_step".to_string(),
            tenant_ref: self.context.tenant_ref.clone(),
            workspace_ref: self.context.workspace_ref.clone(),
            runtime_ref: self.context.runtime_ref.clone(),
            actor_ref: self.context.actor_ref.clone(),
            session_ref: self.context.session_ref.clone(),
            content_hash,
            parent_ref: self.parent.clone(),
            sequence: self.sequence,
            created_at: iso8601_utc(now_unix_ms),
            payload: step.clone(),
        };

        self.parent = Some(receipt_id);

        receipt
    }
}

/// Canonical sha256 of a step's payload.
///
/// serde_json with sorted keys gives a stable byte string for the same
/// logical content, so the hash is reproducible across runs and machines.
fn content_hash(step: &TrainingStep) -> String {
    let canonical = canonical_json(&serde_json::to_value(step).unwrap_or_default());

    let digest = Sha256::digest(canonical.as_bytes());

    hex(&digest)
}

/// serde_json::Value re-serialised with object keys sorted, recursively.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));

            let inner = entries
                .iter()
                .map(|(key, value)| {
                    format!("{}:{}", serde_json::to_string(key).unwrap_or_default(), canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");

            format!("{{{inner}}}")
        }

        serde_json::Value::Array(items) => {
            let inner = items.iter().map(canonical_json).collect::<Vec<_>>().join(",");

            format!("[{inner}]")
        }

        other => other.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Format unix milliseconds as an ISO 8601 UTC string, matching the Forge
/// receipt fixtures (2026-07-25T12:10:00.000Z). Hand-rolled to avoid a date
/// dependency for one formatter.
fn iso8601_utc(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let millis = unix_ms % 1000;

    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;

    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Civil date from days since epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ActionOutcome, PageObservation};

    fn step(action: &str) -> TrainingStep {
        let obs = PageObservation {
            url: "https://example.com/".into(),
            title: "Example".into(),
            text: "hello".into(),
            elements: vec![],
            scroll_y: 0.0,
            scroll_height: 0.0,
        };

        TrainingStep {
            session: "s".into(),
            step: 1,
            elapsed_ms: 0,
            action: action.into(),
            argument: None,
            outcome: ActionOutcome { ok: true, detail: None },
            before: obs.clone(),
            after: obs,
            note: None,
        }
    }

    #[test]
    fn content_hash_is_stable_for_identical_steps() {
        assert_eq!(content_hash(&step("click")), content_hash(&step("click")));
        assert_ne!(content_hash(&step("click")), content_hash(&step("type")));
    }

    #[test]
    fn receipts_form_a_lineage_chain() {
        let mut emitter = ReceiptEmitter::new(ReceiptContext::default());

        let a = emitter.emit(&step("click"), 1_700_000_000_000);
        let b = emitter.emit(&step("type"), 1_700_000_001_000);
        // Two IDENTICAL steps must still get distinct ids and a valid link.
        let c = emitter.emit(&step("click"), 1_700_000_002_000);

        assert_eq!(a.parent_ref, None);
        assert_eq!(a.sequence, 1);
        assert_eq!(b.parent_ref.as_deref(), Some(a.receipt_id.as_str()));
        assert_eq!(b.sequence, 2);
        assert_eq!(c.parent_ref.as_deref(), Some(b.receipt_id.as_str()));
        assert_ne!(c.receipt_id, a.receipt_id);
        // ...but identical content still shares a content_hash for dedup.
        assert_eq!(a.content_hash, c.content_hash);
    }

    #[test]
    fn iso8601_matches_known_instant() {
        // 2023-11-14T22:13:20.000Z
        assert_eq!(iso8601_utc(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
    }
}
