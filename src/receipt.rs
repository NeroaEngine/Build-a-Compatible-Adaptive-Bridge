use crate::router::RouteDecision;
use crate::types::{EngineKind, NodeId};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub receipt_id: Uuid,
    pub timestamp_unix_ms: u64,
    pub node_id: NodeId,
    pub event: String,
    pub engine: Option<EngineKind>,
    pub route: Option<RouteDecision>,
    pub details: Value,
}

impl Receipt {
    pub fn new(
        node_id: NodeId,
        event: impl Into<String>,
        engine: Option<EngineKind>,
        route: Option<RouteDecision>,
        details: Value,
    ) -> Self {
        Self {
            receipt_id: Uuid::new_v4(),
            timestamp_unix_ms: now_ms(),
            node_id,
            event: event.into(),
            engine,
            route,
            details,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("receipt I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("receipt serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub trait ReceiptSink: Send + Sync {
    fn emit(&self, receipt: &Receipt) -> Result<(), ReceiptError>;
}

pub struct JsonlReceiptSink {
    path: PathBuf,
    file: Mutex<File>,
}

impl JsonlReceiptSink {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ReceiptError> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ReceiptSink for JsonlReceiptSink {
    fn emit(&self, receipt: &Receipt) -> Result<(), ReceiptError> {
        let mut file = self.file.lock();

        serde_json::to_writer(&mut *file, receipt)?;
        file.write_all(b"\n")?;
        file.flush()?;

        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryReceiptSink {
    receipts: Mutex<Vec<Receipt>>,
}

impl MemoryReceiptSink {
    pub fn entries(&self) -> Vec<Receipt> {
        self.receipts.lock().clone()
    }
}

impl ReceiptSink for MemoryReceiptSink {
    fn emit(&self, receipt: &Receipt) -> Result<(), ReceiptError> {
        self.receipts.lock().push(receipt.clone());
        Ok(())
    }
}
