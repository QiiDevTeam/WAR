use crate::Target;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollAmount {
    LargeDecrement,
    SmallDecrement,
    NoAmount,
    LargeIncrement,
    SmallIncrement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    Home,
    End,
    Left,
    Right,
    Up,
    Down,
    Character(char),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// A position inside a target's current bounds, where both axes are in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Invoke {
        target: Target,
    },
    SetValue {
        target: Target,
        value: String,
    },
    Toggle {
        target: Target,
        value: Option<bool>,
    },
    Select {
        target: Target,
    },
    Focus {
        target: Target,
    },
    Scroll {
        target: Target,
        amount: ScrollAmount,
    },
    Click {
        target: Target,
        button: MouseButton,
    },
    PointerGesture {
        target: Target,
        button: MouseButton,
        points: Vec<NormalizedPoint>,
        duration_ms: u64,
    },
    TypeText {
        text: String,
    },
    Key {
        key: Key,
        modifiers: Modifiers,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ActionRef<'a> {
    Invoke(&'a Target),
    SetValue {
        target: &'a Target,
        value: &'a str,
    },
    Toggle {
        target: &'a Target,
        value: Option<bool>,
    },
    Select(&'a Target),
    Focus(&'a Target),
    Scroll {
        target: &'a Target,
        amount: ScrollAmount,
    },
    Click {
        target: &'a Target,
        button: MouseButton,
    },
    PointerGesture {
        target: &'a Target,
        button: MouseButton,
        points: &'a [NormalizedPoint],
        duration_ms: u64,
    },
    TypeText(&'a str),
    Key {
        key: &'a Key,
        modifiers: Modifiers,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActionOwned {
    Invoke(Target),
    SetValue {
        target: Target,
        value: String,
    },
    Toggle {
        target: Target,
        value: Option<bool>,
    },
    Select(Target),
    Focus(Target),
    Scroll {
        target: Target,
        amount: ScrollAmount,
    },
    Click {
        target: Target,
        button: MouseButton,
    },
    PointerGesture {
        target: Target,
        button: MouseButton,
        points: Vec<NormalizedPoint>,
        duration_ms: u64,
    },
    TypeText(String),
    Key {
        key: Key,
        modifiers: Modifiers,
    },
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Invoke { target } => ActionRef::Invoke(target),
            Self::SetValue { target, value } => ActionRef::SetValue { target, value },
            Self::Toggle { target, value } => ActionRef::Toggle {
                target,
                value: *value,
            },
            Self::Select { target } => ActionRef::Select(target),
            Self::Focus { target } => ActionRef::Focus(target),
            Self::Scroll { target, amount } => ActionRef::Scroll {
                target,
                amount: *amount,
            },
            Self::Click { target, button } => ActionRef::Click {
                target,
                button: *button,
            },
            Self::PointerGesture {
                target,
                button,
                points,
                duration_ms,
            } => ActionRef::PointerGesture {
                target,
                button: *button,
                points,
                duration_ms: *duration_ms,
            },
            Self::TypeText { text } => ActionRef::TypeText(text),
            Self::Key { key, modifiers } => ActionRef::Key {
                key,
                modifiers: *modifiers,
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match ActionOwned::deserialize(deserializer)? {
            ActionOwned::Invoke(target) => Self::Invoke { target },
            ActionOwned::SetValue { target, value } => Self::SetValue { target, value },
            ActionOwned::Toggle { target, value } => Self::Toggle { target, value },
            ActionOwned::Select(target) => Self::Select { target },
            ActionOwned::Focus(target) => Self::Focus { target },
            ActionOwned::Scroll { target, amount } => Self::Scroll { target, amount },
            ActionOwned::Click { target, button } => Self::Click { target, button },
            ActionOwned::PointerGesture {
                target,
                button,
                points,
                duration_ms,
            } => Self::PointerGesture {
                target,
                button,
                points,
                duration_ms,
            },
            ActionOwned::TypeText(text) => Self::TypeText { text },
            ActionOwned::Key { key, modifiers } => Self::Key { key, modifiers },
        })
    }
}

impl Action {
    pub fn target(&self) -> Option<&Target> {
        match self {
            Self::Invoke { target }
            | Self::SetValue { target, .. }
            | Self::Toggle { target, .. }
            | Self::Select { target }
            | Self::Focus { target }
            | Self::Scroll { target, .. }
            | Self::Click { target, .. } => Some(target),
            Self::PointerGesture { target, .. } => Some(target),
            Self::TypeText { .. } | Self::Key { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum StatePredicate {
    Enabled(bool),
    Focused(bool),
    Selected(bool),
    Checked(bool),
    Expanded(bool),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Condition {
    Exists {
        target: Target,
    },
    Gone {
        target: Target,
    },
    ValueEquals {
        target: Target,
        value: String,
    },
    StateEquals {
        target: Target,
        state: StatePredicate,
    },
    WindowOpened {
        name: String,
    },
    WindowClosed {
        name: String,
    },
    Any {
        conditions: Vec<Condition>,
    },
    All {
        conditions: Vec<Condition>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionBatch {
    /// Opaque session nonce from the snapshot that supplied any `@ref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_session_id: Option<String>,
    /// Epoch the caller observed when choosing any session-local `@ref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_epoch: Option<u64>,
    /// Overall action-loop deadline. Defaults to 15 seconds, maximum 60 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub actions: Vec<Action>,
    pub precondition: Option<Condition>,
    pub postcondition: Option<Condition>,
    #[serde(default = "default_true")]
    pub stop_on_error: bool,
}

impl ActionBatch {
    pub fn uses_refs(&self) -> bool {
        self.actions
            .iter()
            .filter_map(Action::target)
            .any(target_uses_ref)
            || self.precondition.as_ref().is_some_and(condition_uses_ref)
            || self.postcondition.as_ref().is_some_and(condition_uses_ref)
    }
}

fn target_uses_ref(target: &Target) -> bool {
    matches!(target, Target::Ref(_))
}

fn condition_uses_ref(condition: &Condition) -> bool {
    match condition {
        Condition::Exists { target }
        | Condition::Gone { target }
        | Condition::ValueEquals { target, .. }
        | Condition::StateEquals { target, .. } => target_uses_ref(target),
        Condition::Any { conditions } | Condition::All { conditions } => {
            conditions.iter().any(condition_uses_ref)
        }
        Condition::WindowOpened { .. } | Condition::WindowClosed { .. } => false,
    }
}

fn default_true() -> bool {
    true
}
