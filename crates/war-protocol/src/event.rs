use crate::{NodeId, SnapshotDelta, WindowId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    Name,
    Value,
    State,
    Bounds,
    Capabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DesktopEvent {
    NodeAdded {
        node: NodeId,
    },
    NodeRemoved {
        node: NodeId,
    },
    PropertyChanged {
        node: NodeId,
        property: Property,
    },
    FocusChanged {
        from: Option<NodeId>,
        to: Option<NodeId>,
    },
    WindowOpened {
        window: WindowId,
    },
    WindowClosed {
        window: WindowId,
    },
    Delta {
        delta: SnapshotDelta,
    },
}
