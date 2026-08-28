use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::WindowId;

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "message")]
pub enum WarError {
    #[error("target not found: {0}")]
    TargetNotFound(String),
    #[error("target resolution confidence is too low: {0:.2}")]
    LowConfidence(f32),
    #[error("capability is not available: {0}")]
    CapabilityUnavailable(String),
    #[error("precondition failed: {0}")]
    PreconditionFailed(String),
    #[error("postcondition failed: {0}")]
    PostconditionFailed(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("{operation} timed out after {timeout_ms} ms")]
    Timeout { operation: String, timeout_ms: u64 },
    #[error("stale snapshot: expected epoch {expected}, current epoch is {current}")]
    StaleSnapshot { expected: u64, current: u64 },
    #[error("stale session: expected {expected}, current session is {current}")]
    StaleSession { expected: String, current: String },
    #[error(
        "global input refused: target process {target_process} is not foreground process {foreground_process}"
    )]
    ForegroundMismatch {
        target_process: u32,
        foreground_process: u32,
    },
    #[error(
        "mouse input refused: target window {target_window} is obscured by window {hit_window}"
    )]
    HitTestMismatch {
        target_window: WindowId,
        hit_window: WindowId,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),
}

pub type WarResult<T> = Result<T, WarError>;
