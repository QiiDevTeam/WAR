use war_protocol::{Action, ActionBatch, Capabilities, WarError, WarResult};

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub index: usize,
    /// The provider accepted and dispatched the action. This does not prove a UI effect.
    pub dispatched: bool,
    pub method: Option<String>,
    pub fallback_used: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Verified,
    DispatchedUnverified,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEffect {
    Changed,
    NoChange,
}

#[derive(Debug, Clone)]
pub struct BatchOutcome {
    pub actions: Vec<ActionOutcome>,
    pub verified: Option<bool>,
    pub status: ExecutionStatus,
    pub effect: ObservedEffect,
}

pub fn required_capability(action: &Action) -> Option<&'static str> {
    match action {
        Action::Invoke { .. } => Some("invoke"),
        Action::SetValue { .. } => Some("set_value"),
        Action::Toggle { .. } => Some("toggle"),
        Action::Select { .. } => Some("select"),
        Action::Focus { .. } => Some("focus"),
        Action::Scroll { .. } => Some("scroll"),
        Action::Click { .. } => Some("click"),
        Action::PointerGesture { .. } => Some("pointer_gesture"),
        Action::TypeText { .. } => Some("type_text"),
        Action::Key { .. } => None,
    }
}

pub fn required_capability_flag(action: &Action) -> Option<Capabilities> {
    match action {
        Action::Invoke { .. } => Some(Capabilities::INVOKE),
        Action::SetValue { .. } => Some(Capabilities::SET_VALUE),
        Action::Toggle { .. } => Some(Capabilities::TOGGLE),
        Action::Select { .. } => Some(Capabilities::SELECT),
        Action::Focus { .. } => Some(Capabilities::FOCUS),
        Action::Scroll { .. } => Some(Capabilities::SCROLL),
        Action::Click { .. } => Some(Capabilities::CLICK),
        Action::PointerGesture { .. } => Some(Capabilities::POINTER_GESTURE),
        Action::TypeText { .. } => Some(Capabilities::TYPE_TEXT),
        Action::Key { .. } => None,
    }
}

pub fn validate_batch(batch: &ActionBatch) -> WarResult<()> {
    if batch.actions.is_empty() {
        return Err(WarError::InvalidRequest("action batch is empty".into()));
    }
    if batch.actions.len() > 128 {
        return Err(WarError::InvalidRequest(
            "action batch exceeds 128 actions".into(),
        ));
    }
    if batch
        .timeout_ms
        .is_some_and(|timeout| timeout == 0 || timeout > 60_000)
    {
        return Err(WarError::InvalidRequest(
            "action batch timeout_ms must be between 1 and 60000".into(),
        ));
    }
    for action in &batch.actions {
        let text = match action {
            Action::SetValue { value, .. } => Some(value),
            Action::TypeText { text } => Some(text),
            _ => None,
        };
        if text.is_some_and(|text| text.len() > 64 * 1024) {
            return Err(WarError::InvalidRequest(
                "action text exceeds the 64 KiB limit".into(),
            ));
        }
        if let Action::PointerGesture {
            target,
            points,
            duration_ms,
            ..
        } = action
        {
            if matches!(target, war_protocol::Target::Coordinates(_)) {
                return Err(WarError::InvalidRequest(
                    "pointer_gesture requires an element target".into(),
                ));
            }
            if !(2..=4096).contains(&points.len()) {
                return Err(WarError::InvalidRequest(
                    "pointer_gesture requires between 2 and 4096 points".into(),
                ));
            }
            if *duration_ms == 0 || *duration_ms > 60_000 {
                return Err(WarError::InvalidRequest(
                    "pointer_gesture duration_ms must be between 1 and 60000".into(),
                ));
            }
            if points.iter().any(|point| {
                !point.x.is_finite()
                    || !point.y.is_finite()
                    || !(0.0..=1.0).contains(&point.x)
                    || !(0.0..=1.0).contains(&point.y)
            }) {
                return Err(WarError::InvalidRequest(
                    "pointer_gesture points must use finite normalized coordinates".into(),
                ));
            }
        }
    }
    Ok(())
}
