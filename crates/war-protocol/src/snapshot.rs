use crate::{Capabilities, DesktopNode, NodeId, NodeStates, Role, WindowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSnapshot {
    pub epoch: u64,
    pub root: NodeId,
    pub nodes: HashMap<NodeId, DesktopNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub process_id: u32,
    pub app: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNode {
    pub id: NodeId,
    pub role: Role,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub value: Option<String>,
    pub states: NodeStates,
    pub capabilities: Capabilities,
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticSnapshot {
    /// Opaque nonce identifying the runtime session that owns every `@ref`.
    /// Empty only for snapshots compiled outside a runtime (primarily tests).
    #[serde(default)]
    pub session_id: String,
    pub epoch: u64,
    pub window: WindowInfo,
    pub nodes: Vec<SemanticNode>,
    /// Number of semantic nodes before the agent-facing budget was applied.
    #[serde(default)]
    pub total_nodes: usize,
    /// True when lower-relevance nodes were omitted from this response.
    #[serde(default)]
    pub truncated: bool,
    pub focused: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeChange {
    pub id: NodeId,
    pub before: SemanticNode,
    pub after: SemanticNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusChange {
    pub from: Option<NodeId>,
    pub to: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDelta {
    pub from_epoch: u64,
    pub to_epoch: u64,
    pub added: Vec<SemanticNode>,
    pub removed: Vec<NodeId>,
    pub changed: Vec<NodeChange>,
    pub focus_changed: Option<FocusChange>,
}
