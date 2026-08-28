use crate::{Capabilities, NodeId, Point, Role, WindowId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SnapshotScope {
    Desktop,
    Process(u32),
    Window(WindowId),
    Node(NodeId),
    FocusedWindow,
    FocusedSubtree,
}

impl Default for SnapshotScope {
    fn default() -> Self {
        Self::FocusedWindow
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticTarget {
    pub role: Option<Role>,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Capabilities::is_empty")]
    pub required_capabilities: Capabilities,
    pub ancestor: Option<Box<SemanticTarget>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Ref(NodeId),
    Semantic(SemanticTarget),
    Coordinates(Point),
}

impl Target {
    pub fn parse_ref(value: &str) -> Option<Self> {
        value
            .strip_prefix('@')
            .and_then(|v| v.parse().ok())
            .map(Self::Ref)
    }
}

impl Serialize for Target {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire<'a> {
            Semantic(&'a SemanticTarget),
            Coordinates(&'a Point),
        }
        match self {
            Self::Ref(id) => serializer.serialize_str(&format!("@{id}")),
            Self::Semantic(value) => Wire::Semantic(value).serialize(serializer),
            Self::Coordinates(value) => Wire::Coordinates(value).serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", untagged)]
        enum Wire {
            Ref(String),
            Semantic { semantic: SemanticTarget },
            Coordinates { coordinates: Point },
        }
        match Wire::deserialize(deserializer)? {
            Wire::Ref(value) => Target::parse_ref(&value)
                .ok_or_else(|| serde::de::Error::custom("node ref must look like @12")),
            Wire::Semantic { semantic } => Ok(Self::Semantic(semantic)),
            Wire::Coordinates { coordinates } => Ok(Self::Coordinates(coordinates)),
        }
    }
}
