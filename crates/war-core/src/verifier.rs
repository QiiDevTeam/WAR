use crate::resolve_target;
use war_protocol::{Condition, SemanticNode, SemanticSnapshot, StatePredicate, Target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub success: bool,
    pub detail: String,
}

pub trait Verifier: Send + Sync {
    fn verify(
        &self,
        before: &SemanticSnapshot,
        after: &SemanticSnapshot,
        expectation: &Condition,
    ) -> Verification;
}

#[derive(Debug, Default)]
pub struct LocalVerifier;

impl Verifier for LocalVerifier {
    fn verify(
        &self,
        before: &SemanticSnapshot,
        after: &SemanticSnapshot,
        condition: &Condition,
    ) -> Verification {
        let success = evaluate(before, after, condition);
        Verification {
            success,
            detail: if success {
                "condition satisfied".into()
            } else {
                format!("condition not satisfied: {condition:?}")
            },
        }
    }
}

fn node<'a>(snapshot: &'a SemanticSnapshot, target: &Target) -> Option<&'a SemanticNode> {
    let id = resolve_target(snapshot, target).ok()?.id;
    snapshot.nodes.iter().find(|node| node.id == id)
}

fn evaluate(before: &SemanticSnapshot, after: &SemanticSnapshot, condition: &Condition) -> bool {
    match condition {
        Condition::Exists { target } => node(after, target).is_some(),
        Condition::Gone { target } => node(after, target).is_none(),
        Condition::ValueEquals { target, value } => {
            node(after, target).and_then(|n| n.value.as_ref()) == Some(value)
        }
        Condition::StateEquals { target, state } => {
            node(after, target).is_some_and(|n| match state {
                StatePredicate::Enabled(v) => n.states.enabled == *v,
                StatePredicate::Focused(v) => n.states.focused == *v,
                StatePredicate::Selected(v) => n.states.selected == Some(*v),
                StatePredicate::Checked(v) => n.states.checked == Some(*v),
                StatePredicate::Expanded(v) => n.states.expanded == Some(*v),
            })
        }
        Condition::WindowOpened { name } => !has_window(before, name) && has_window(after, name),
        Condition::WindowClosed { name } => has_window(before, name) && !has_window(after, name),
        Condition::Any { conditions } => conditions.iter().any(|c| evaluate(before, after, c)),
        Condition::All { conditions } => conditions.iter().all(|c| evaluate(before, after, c)),
    }
}

fn has_window(snapshot: &SemanticSnapshot, name: &str) -> bool {
    snapshot.window.title.as_deref() == Some(name)
        || snapshot.nodes.iter().any(|node| {
            matches!(
                node.role,
                war_protocol::Role::Window | war_protocol::Role::Dialog
            ) && node.name.as_deref() == Some(name)
        })
}
