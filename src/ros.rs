//! Receipt OS client.
//!
//! NEROA_ROS_CLIENT_V14
//!
//! Per the ROS handoff (os_authority/docs/ROS-HANDOFF-joining.md), the entire
//! surface is two calls:
//!
//!   POST /v1/receipts        {level, kind, evidence, submittedBy} -> 201 + ref
//!   POST /v1/receipts/verify {ref}                                -> 200/409/404
//!
//! What this deliberately does NOT do, because the handoff forbids it:
//!  * No local hash chain, no content digest, no ref minting. ROS computes the
//!    digest and returns the ref. A second history is a history that disagrees.
//!  * No caller-chosen payload shape. Exactly {level, kind, evidence,
//!    submittedBy} goes on the wire.
//!  * No caller-supplied timestamp. ROS stamps `at`.
//!  * No blind retry. A duplicate receipt is a real receipt; verify the ref
//!    instead.
//!
//! `submittedBy` is load-bearing identity, not decoration: it becomes this
//! engine's interleaving writers in the ledger. It is fixed to a stable,
//! boring name and never varies.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};

/// Governance levels. The level is a decision about blast radius - what the
/// evidence is worth in a dispute - not a property of the emitting code, so it
/// is always chosen explicitly at the call site and never inferred here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Ordinary application events - most of what a presentation surface emits.
    Application = 1,
    /// Reads and writes crossing a boundary.
    DataIo = 2,
    /// Build and artifact provenance.
    Build = 3,
    /// Runtime and boot events.
    KernelBoot = 4,
    /// Authority decisions. Never pruned.
    Authority = 5,
}

impl Level {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// What a submitted reference is worth at the moment it was returned.
///
/// `in_ledger` is not `witnessed`. The handoff is explicit: do not render
/// "accepted" as "witnessed" anywhere.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Proven {
    #[serde(rename = "inLedger", default)]
    pub in_ledger: bool,

    #[serde(default)]
    pub sealed: bool,

    #[serde(default)]
    pub witnessed: bool,
}

/// The reference ROS assigns. The engine does not choose it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceiptRef {
    pub ref_: String,

    #[serde(default)]
    pub seq: Option<u64>,

    #[serde(default)]
    pub writer: Option<String>,

    #[serde(default)]
    pub proven: Proven,
}

/// Verify verdicts are deliberately distinct and must not be collapsed:
/// 409 (exists, does not check out - somebody may be lying) is not 404
/// (never heard of it - you have a typo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    Invalid,
    NotFound,
    Unexpected(u16),
}

#[derive(Debug)]
pub enum RosError {
    Http(String),
    Rejected { status: u16, body: String },
    NoToken,
}

impl std::fmt::Display for RosError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RosError::Http(message) => write!(f, "ros http: {message}"),
            RosError::Rejected { status, body } => write!(f, "ros rejected {status}: {body}"),
            RosError::NoToken => write!(f, "ros: no bearer token available"),
        }
    }
}

impl std::error::Error for RosError {}

/// A ROS client bound to one base URL and one identity.
#[derive(Clone)]
pub struct RosClient {
    base_url: String,
    token: String,
    submitted_by: String,
}

#[derive(Serialize)]
struct SubmitBody<'a> {
    level: u8,
    kind: &'a str,
    evidence: serde_json::Value,
    #[serde(rename = "submittedBy")]
    submitted_by: &'a str,
}

#[derive(Deserialize)]
struct SubmitResponse {
    #[serde(rename = "ref")]
    ref_: String,
    #[serde(default)]
    seq: Option<u64>,
    #[serde(default)]
    writer: Option<String>,
    #[serde(default)]
    proven: Proven,
}

impl RosClient {
    /// `base_url` like `http://127.0.0.1:8770`. `submitted_by` is this engine's
    /// permanent identity - lower-cased and bounded by ROS, so pick it once.
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        submitted_by: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            submitted_by: submitted_by.into(),
        }
    }

    pub fn submitted_by(&self) -> &str {
        &self.submitted_by
    }

    /// Submit a receipt. `evidence` is the engine's domain payload, untouched;
    /// ROS is deliberately incurious about it.
    pub async fn submit(
        &self,
        level: Level,
        kind: &str,
        evidence: serde_json::Value,
    ) -> Result<ReceiptRef, RosError> {
        if self.token.is_empty() {
            return Err(RosError::NoToken);
        }

        let body = SubmitBody {
            level: level.as_u8(),
            kind,
            evidence,
            submitted_by: &self.submitted_by,
        };

        let payload = serde_json::to_vec(&body)
            .map_err(|error| RosError::Http(format!("encode: {error}")))?;

        let (status, bytes) = self
            .post("/v1/receipts", payload)
            .await?;

        if status != 200 && status != 201 {
            return Err(RosError::Rejected {
                status,
                body: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let response: SubmitResponse = serde_json::from_slice(&bytes).map_err(|error| {
            RosError::Http(format!(
                "decode: {error}: {}",
                String::from_utf8_lossy(&bytes)
            ))
        })?;

        Ok(ReceiptRef {
            ref_: response.ref_,
            seq: response.seq,
            writer: response.writer,
            proven: response.proven,
        })
    }

    /// Verify a reference. Returns the distinct verdict; the caller must keep
    /// 409 and 404 apart.
    pub async fn verify(&self, ros_ref: &str) -> Result<Verdict, RosError> {
        let payload = serde_json::to_vec(&serde_json::json!({ "ref": ros_ref }))
            .map_err(|error| RosError::Http(format!("encode: {error}")))?;

        let (status, _bytes) = self.post("/v1/receipts/verify", payload).await?;

        Ok(match status {
            200 => Verdict::Valid,
            409 => Verdict::Invalid,
            404 => Verdict::NotFound,
            other => Verdict::Unexpected(other),
        })
    }

    async fn post(&self, path: &str, payload: Vec<u8>) -> Result<(u16, Bytes), RosError> {
        let connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();

        let client: Client<_, Full<Bytes>> =
            Client::builder(TokioExecutor::new()).build(connector);

        let request = Request::builder()
            .method("POST")
            .uri(format!("{}{path}", self.base_url))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("authorization", format!("Bearer {}", self.token))
            .body(Full::new(Bytes::from(payload)))
            .map_err(|error| RosError::Http(format!("request build: {error}")))?;

        let response = client
            .request(request)
            .await
            .map_err(|error| RosError::Http(format!("send: {error}")))?;

        let status = response.status().as_u16();

        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| RosError::Http(format!("read body: {error}")))?
            .to_bytes();

        Ok((status, bytes))
    }
}

/// Resolve a bearer token without ever taking it on argv.
///
/// Order matches the handoff's preference: an explicit env var name, then a
/// file. The vault Unix-socket resolver is OVH-only and not reachable from a
/// Windows presentation cell, so it is not attempted here; on that host the
/// token comes from the deployment's env or a file.
pub fn resolve_token(env_name: Option<&str>, file_path: Option<&str>) -> Option<String> {
    if let Some(name) = env_name {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }

    if let Some(path) = file_path {
        if let Ok(value) = std::fs::read_to_string(path) {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_exactly_the_ros_shape() {
        let body = SubmitBody {
            level: Level::Application.as_u8(),
            kind: "spatial.frame.rendered",
            evidence: serde_json::json!({ "scene": "x" }),
            submitted_by: "spatial-browser",
        };

        let json: serde_json::Value = serde_json::to_value(&body).unwrap();

        // The four fields the handoff specifies, and only those.
        let keys: std::collections::BTreeSet<_> =
            json.as_object().unwrap().keys().cloned().collect();

        let expected: std::collections::BTreeSet<_> =
            ["level", "kind", "evidence", "submittedBy"]
                .into_iter()
                .map(str::to_string)
                .collect();

        assert_eq!(keys, expected);
        assert_eq!(json["level"], 1);
        assert_eq!(json["submittedBy"], "spatial-browser");
    }

    #[test]
    fn levels_map_to_their_numbers() {
        assert_eq!(Level::Application.as_u8(), 1);
        assert_eq!(Level::DataIo.as_u8(), 2);
        assert_eq!(Level::Authority.as_u8(), 5);
    }
}
