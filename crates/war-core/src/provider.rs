use crossbeam_channel::Receiver;
use std::time::Duration;
use war_protocol::{
    Action, DesktopEvent, NodeId, NodeSource, RawSnapshot, SnapshotScope, WarResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderNodeRef {
    pub id: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderActionResult {
    pub method: String,
    pub fallback_used: bool,
}

pub struct Subscription {
    receiver: Receiver<DesktopEvent>,
}

impl Subscription {
    pub fn new(receiver: Receiver<DesktopEvent>) -> Self {
        Self { receiver }
    }
    pub fn receiver(&self) -> &Receiver<DesktopEvent> {
        &self.receiver
    }
}

pub trait DesktopProvider: Send + Sync {
    fn kind(&self) -> NodeSource;
    fn snapshot(&self, scope: SnapshotScope) -> WarResult<RawSnapshot>;
    fn snapshot_with_timeout(
        &self,
        scope: SnapshotScope,
        _timeout: Duration,
    ) -> WarResult<RawSnapshot> {
        self.snapshot(scope)
    }
    fn execute(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
    ) -> WarResult<ProviderActionResult>;
    fn execute_with_timeout(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
        _timeout: Duration,
    ) -> WarResult<ProviderActionResult> {
        self.execute(node, action)
    }
    fn subscribe(&self) -> WarResult<Subscription>;
}
