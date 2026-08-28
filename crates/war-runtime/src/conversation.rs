use crate::{ExecutionReport, WarRuntime};
use std::time::{Duration, Instant};
use war_core::{nearest_capable_ancestor, resolve_target};
use war_protocol::{
    Action, ActionBatch, Capabilities, Condition, MouseButton, Role, SemanticSnapshot,
    SemanticTarget, SendMessageReport, SendMessageRequest, Target, WarError, WarResult,
};

const INVOKE_PROBE_MS: u64 = 350;

impl WarRuntime {
    /// Sends one message while keeping all intermediate snapshots inside the runtime.
    pub fn send_message(&self, request: &SendMessageRequest) -> WarResult<SendMessageReport> {
        validate_request(request)?;
        let started = Instant::now();
        let deadline = started + Duration::from_millis(request.timeout_ms);
        let mut observations = 1;
        let mut actions = 0;
        let first = self.observe_with_timeout(
            request.scope.clone(),
            remaining(deadline, request.timeout_ms)?,
        )?;
        let editor = editor_target(&request.recipient);
        let activation_condition = Condition::Exists {
            target: Target::Semantic(editor.clone()),
        };

        let recipient_id = resolve_target(&first, &Target::Semantic(recipient_target(request)))?.id;

        let (activation, activated) =
            match nearest_capable_ancestor(&first, recipient_id, Capabilities::INVOKE) {
                Ok(node) => {
                    actions += 1;
                    let probe_timeout =
                        remaining_ms(deadline, request.timeout_ms)?.min(INVOKE_PROBE_MS);
                    match self.run_step(
                        &first,
                        Action::Invoke {
                            target: Target::Ref(node.id),
                        },
                        activation_condition.clone(),
                        probe_timeout,
                    ) {
                        Ok(report) => ("invoke".into(), report),
                        Err(WarError::PostconditionFailed(_)) => {
                            let current = self.current()?.ok_or_else(|| {
                                WarError::Provider("activation probe lost current snapshot".into())
                            })?;
                            let current_recipient = resolve_target(
                                &current,
                                &Target::Semantic(recipient_target(request)),
                            )?
                            .id;
                            let clickable = nearest_capable_ancestor(
                                &current,
                                current_recipient,
                                Capabilities::CLICK,
                            )?;
                            actions += 1;
                            let report = self.run_step(
                                &current,
                                Action::Click {
                                    target: Target::Ref(clickable.id),
                                    button: MouseButton::Left,
                                },
                                activation_condition,
                                remaining_ms(deadline, request.timeout_ms)?,
                            )?;
                            ("click_fallback".into(), report)
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(_) => {
                    let clickable =
                        nearest_capable_ancestor(&first, recipient_id, Capabilities::CLICK)?;
                    actions += 1;
                    let report = self.run_step(
                        &first,
                        Action::Click {
                            target: Target::Ref(clickable.id),
                            button: MouseButton::Left,
                        },
                        activation_condition,
                        remaining_ms(deadline, request.timeout_ms)?,
                    )?;
                    ("click".into(), report)
                }
            };
        observations += activated.observations;

        let current = activated.snapshot;
        let editor_id = resolve_target(&current, &Target::Semantic(editor.clone()))?.id;
        let send_target = SemanticTarget {
            role: Some(Role::Button),
            name: Some(request.send_label.clone()),
            automation_id: None,
            required_capabilities: Capabilities::INVOKE,
            ancestor: None,
        };
        actions += 1;
        let drafted = self.run_step(
            &current,
            Action::SetValue {
                target: Target::Ref(editor_id),
                value: request.text.clone(),
            },
            Condition::All {
                conditions: vec![
                    Condition::ValueEquals {
                        target: Target::Ref(editor_id),
                        value: request.text.clone(),
                    },
                    Condition::Exists {
                        target: Target::Semantic(send_target.clone()),
                    },
                ],
            },
            remaining_ms(deadline, request.timeout_ms)?,
        )?;
        observations += drafted.observations;

        let before_count = message_count(&drafted.snapshot, &request.text);
        let send_id = resolve_target(&drafted.snapshot, &Target::Semantic(send_target))?.id;
        actions += 1;
        let sent = self.run_step(
            &drafted.snapshot,
            Action::Invoke {
                target: Target::Ref(send_id),
            },
            Condition::ValueEquals {
                target: Target::Ref(editor_id),
                value: String::new(),
            },
            remaining_ms(deadline, request.timeout_ms)?,
        )?;
        observations += sent.observations;
        if message_count(&sent.snapshot, &request.text) <= before_count {
            return Err(WarError::PostconditionFailed(
                "composer cleared, but no new outgoing message became visible".into(),
            ));
        }

        Ok(SendMessageReport {
            status: "verified".into(),
            recipient: request.recipient.clone(),
            activation,
            actions,
            observations,
            elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        })
    }

    fn run_step(
        &self,
        snapshot: &SemanticSnapshot,
        action: Action,
        postcondition: Condition,
        timeout_ms: u64,
    ) -> WarResult<ExecutionReport> {
        self.act(&ActionBatch {
            expected_session_id: Some(snapshot.session_id.clone()),
            expected_epoch: Some(snapshot.epoch),
            timeout_ms: Some(timeout_ms.clamp(1, 60_000)),
            actions: vec![action],
            precondition: None,
            postcondition: Some(postcondition),
            stop_on_error: true,
        })
    }
}

fn editor_target(recipient: &str) -> SemanticTarget {
    SemanticTarget {
        role: None,
        name: Some(recipient.into()),
        automation_id: None,
        required_capabilities: Capabilities::SET_VALUE,
        ancestor: None,
    }
}

fn recipient_target(request: &SendMessageRequest) -> SemanticTarget {
    SemanticTarget {
        role: Some(Role::Text),
        name: Some(request.recipient.clone()),
        automation_id: None,
        required_capabilities: Capabilities::empty(),
        ancestor: Some(Box::new(SemanticTarget {
            role: None,
            name: Some(request.list_name.clone()),
            automation_id: None,
            required_capabilities: Capabilities::empty(),
            ancestor: None,
        })),
    }
}

fn message_count(snapshot: &SemanticSnapshot, text: &str) -> usize {
    snapshot
        .nodes
        .iter()
        .filter(|node| node.role == Role::Text && node.name.as_deref() == Some(text))
        .count()
}

fn validate_request(request: &SendMessageRequest) -> WarResult<()> {
    if request.recipient.trim().is_empty()
        || request.list_name.trim().is_empty()
        || request.send_label.trim().is_empty()
    {
        return Err(WarError::InvalidRequest(
            "recipient, list_name, and send_label must not be empty".into(),
        ));
    }
    if request.text.is_empty() || request.text.len() > 64 * 1024 {
        return Err(WarError::InvalidRequest(
            "message text must be between 1 byte and 64 KiB".into(),
        ));
    }
    if request.timeout_ms == 0 || request.timeout_ms > 60_000 {
        return Err(WarError::InvalidRequest(
            "timeout_ms must be between 1 and 60000".into(),
        ));
    }
    Ok(())
}

fn remaining(deadline: Instant, timeout_ms: u64) -> WarResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| WarError::Timeout {
            operation: "send_message workflow".into(),
            timeout_ms,
        })
}

fn remaining_ms(deadline: Instant, timeout_ms: u64) -> WarResult<u64> {
    let remaining = remaining(deadline, timeout_ms)?;
    Ok(remaining.as_millis().try_into().unwrap_or(u64::MAX).max(1))
}
