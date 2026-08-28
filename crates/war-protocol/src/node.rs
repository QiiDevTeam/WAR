use bitflags::bitflags;
use serde::{Deserialize, Serialize};

pub type NodeId = u64;
pub type WindowId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeSource {
    Uia,
    Win32,
    Java,
    Vision,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Window,
    Dialog,
    Button,
    Toggle,
    Checkbox,
    Radio,
    Text,
    Label,
    Heading,
    TextInput,
    TextArea,
    ComboBox,
    List,
    ListItem,
    Tree,
    TreeItem,
    Menu,
    MenuItem,
    Tab,
    TabItem,
    Toolbar,
    Slider,
    ProgressBar,
    Document,
    Canvas,
    Image,
    Link,
    Pane,
    Group,
    Unknown,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format!("{self:?}").to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn center(self) -> Point {
        Point {
            x: self.left + self.width / 2.0,
            y: self.top + self.height / 2.0,
        }
    }
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStates {
    pub enabled: bool,
    pub focused: bool,
    pub focusable: bool,
    pub offscreen: bool,
    pub selected: Option<bool>,
    pub checked: Option<bool>,
    pub expanded: Option<bool>,
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capabilities: u32 {
        const INVOKE = 1 << 0;
        const SET_VALUE = 1 << 1;
        const GET_VALUE = 1 << 2;
        const TOGGLE = 1 << 3;
        const SELECT = 1 << 4;
        const EXPAND = 1 << 5;
        const COLLAPSE = 1 << 6;
        const SCROLL = 1 << 7;
        const FOCUS = 1 << 8;
        const CLICK = 1 << 9;
        const TYPE_TEXT = 1 << 10;
        const POINTER_GESTURE = 1 << 11;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeFingerprint {
    pub process_id: u32,
    pub role: Role,
    pub automation_id: Option<u64>,
    pub name_hash: Option<u64>,
    pub ancestor_hash: u64,
    pub sibling_hint: u16,
}

impl Default for Role {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopNode {
    pub id: NodeId,
    pub source: NodeSource,
    pub role: Role,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub bounds: Option<Rect>,
    pub states: NodeStates,
    pub capabilities: Capabilities,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub fingerprint: NodeFingerprint,
}
