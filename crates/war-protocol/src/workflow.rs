use crate::SnapshotScope;
use serde::{Deserialize, Serialize};

fn default_workflow_timeout_ms() -> u64 {
    15_000
}

fn default_send_label() -> String {
    "发送".into()
}

fn default_list_name() -> String {
    "会话列表".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendMessageRequest {
    #[serde(default)]
    pub scope: SnapshotScope,
    pub recipient: String,
    pub text: String,
    #[serde(default = "default_list_name")]
    pub list_name: String,
    #[serde(default = "default_send_label")]
    pub send_label: String,
    #[serde(default = "default_workflow_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessageReport {
    pub status: String,
    pub recipient: String,
    pub activation: String,
    pub actions: u32,
    pub observations: u32,
    pub elapsed_ms: u64,
}
