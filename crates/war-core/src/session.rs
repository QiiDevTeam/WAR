use crate::ProviderNodeRef;
use std::collections::HashMap;
use war_protocol::{NodeId, Rect, SemanticSnapshot, SnapshotScope, WindowInfo};

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub semantic: SemanticSnapshot,
    pub provider_refs: HashMap<NodeId, ProviderNodeRef>,
    pub bounds: HashMap<NodeId, Rect>,
    pub scope: SnapshotScope,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub window: WindowInfo,
    pub current: Option<SessionSnapshot>,
}

impl Session {
    pub fn new(id: String) -> Self {
        Self {
            id,
            window: WindowInfo::default(),
            current: None,
        }
    }
}
