use crate::{observe::poisoned, WarRuntime};
use war_core::{
    required_capability, required_capability_flag, resolve_target, validate_batch, ActionOutcome,
    BatchOutcome, ExecutionStatus, ObservedEffect, Verifier,
};
use war_protocol::{ActionBatch, SnapshotDelta, SnapshotScope, Target, WarError, WarResult};
use war_semantic::diff;

const DEFAULT_BATCH_TIMEOUT_MS: u64 = 15_000;

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub outcome: BatchOutcome,
    pub snapshot: war_protocol::SemanticSnapshot,
    pub delta: SnapshotDelta,
    pub observations: u32,
}

impl WarRuntime {
    pub fn act(&self, batch: &ActionBatch) -> WarResult<ExecutionReport> {
        validate_batch(batch)?;
        let batch_timeout_ms = batch.timeout_ms.unwrap_or(DEFAULT_BATCH_TIMEOUT_MS);
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(batch_timeout_ms);
        if self.current()?.is_none() {
            self.observe_with_timeout(
                SnapshotScope::FocusedWindow,
                remaining(deadline, batch_timeout_ms)?,
            )?;
        }
        let (before, scope, provider_refs) = {
            let session = self.session.lock().map_err(poisoned)?;
            let current = session.current.as_ref().ok_or_else(|| {
                WarError::Provider("runtime has no current snapshot after observe".into())
            })?;
            (
                current.semantic.clone(),
                current.scope.clone(),
                current.provider_refs.clone(),
            )
        };
        if batch.uses_refs() {
            let expected_session = batch.expected_session_id.as_deref().ok_or_else(|| {
                WarError::InvalidRequest(
                    "action batch with session refs requires expected_session_id".into(),
                )
            })?;
            if expected_session != before.session_id {
                return Err(WarError::StaleSession {
                    expected: expected_session.into(),
                    current: before.session_id.clone(),
                });
            }
            if batch.expected_epoch.is_none() {
                return Err(WarError::InvalidRequest(
                    "action batch with session refs requires expected_epoch".into(),
                ));
            }
        }
        if let Some(expected) = batch.expected_epoch {
            if expected != before.epoch {
                return Err(WarError::StaleSnapshot {
                    expected,
                    current: before.epoch,
                });
            }
        }
        if let Some(condition) = &batch.precondition {
            let verification = self.verifier.verify(&before, &before, condition);
            if !verification.success {
                return Err(WarError::PreconditionFailed(verification.detail));
            }
        }
        let mut outcomes = Vec::with_capacity(batch.actions.len());
        let subscription = batch
            .postcondition
            .as_ref()
            .and_then(|_| self.provider.subscribe().ok());
        for (index, action) in batch.actions.iter().enumerate() {
            let action_timeout = remaining(deadline, batch_timeout_ms)?;
            if uses_unscoped_input(action) {
                ensure_foreground_process(before.window.process_id)?;
            }
            let provider_ref = if let Some(target) = action.target() {
                match target {
                    Target::Coordinates(_) => None,
                    _ => {
                        let stable = resolve_target(&before, target)?.id;
                        ensure_capability(&before, stable, action)?;
                        provider_refs
                            .get(&stable)
                            .copied()
                            .ok_or_else(|| {
                                WarError::TargetNotFound(format!("provider mapping for @{stable}"))
                            })
                            .map(Some)?
                    }
                }
            } else {
                if matches!(action, war_protocol::Action::TypeText { .. }) {
                    let focused = before.focused.ok_or_else(|| {
                        WarError::CapabilityUnavailable(
                            "type_text requires a focused editable node".into(),
                        )
                    })?;
                    ensure_capability(&before, focused, action)?;
                }
                None
            };
            match self
                .provider
                .execute_with_timeout(provider_ref, action, action_timeout)
            {
                Ok(result) => outcomes.push(ActionOutcome {
                    index,
                    dispatched: true,
                    method: Some(result.method),
                    fallback_used: Some(result.fallback_used),
                    error: None,
                }),
                Err(error) => {
                    outcomes.push(ActionOutcome {
                        index,
                        dispatched: false,
                        method: None,
                        fallback_used: None,
                        error: Some(error.to_string()),
                    });
                    if batch.stop_on_error {
                        break;
                    }
                }
            }
        }
        let (after, observations) = self.observe_until(
            &before,
            scope,
            batch.postcondition.as_ref(),
            subscription.as_ref(),
            deadline,
            batch_timeout_ms,
        )?;
        let delta = diff(&before, &after);
        let verified = batch
            .postcondition
            .as_ref()
            .map(|condition| self.verifier.verify(&before, &after, condition).success);
        if verified == Some(false) {
            return Err(WarError::PostconditionFailed(format!(
                "{:?}",
                batch.postcondition
            )));
        }
        let all_dispatched = outcomes.len() == batch.actions.len()
            && outcomes.iter().all(|outcome| outcome.dispatched);
        let status = if !all_dispatched {
            ExecutionStatus::Failed
        } else if verified == Some(true) {
            ExecutionStatus::Verified
        } else {
            ExecutionStatus::DispatchedUnverified
        };
        let effect = if delta.added.is_empty()
            && delta.removed.is_empty()
            && delta.changed.is_empty()
            && delta.focus_changed.is_none()
        {
            ObservedEffect::NoChange
        } else {
            ObservedEffect::Changed
        };
        Ok(ExecutionReport {
            outcome: BatchOutcome {
                actions: outcomes,
                verified,
                status,
                effect,
            },
            snapshot: after,
            delta,
            observations,
        })
    }

    fn observe_until(
        &self,
        before: &war_protocol::SemanticSnapshot,
        scope: SnapshotScope,
        postcondition: Option<&war_protocol::Condition>,
        subscription: Option<&war_core::Subscription>,
        deadline: std::time::Instant,
        timeout_ms: u64,
    ) -> WarResult<(war_protocol::SemanticSnapshot, u32)> {
        let mut observations = 0;
        loop {
            let after =
                self.observe_with_timeout(scope.clone(), remaining(deadline, timeout_ms)?)?;
            observations += 1;
            let Some(condition) = postcondition else {
                return Ok((after, observations));
            };
            if self.verifier.verify(before, &after, condition).success {
                return Ok((after, observations));
            }
            let Some(time_left) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return Ok((after, observations));
            };
            if time_left.is_zero() {
                return Ok((after, observations));
            }
            let pause = time_left.min(std::time::Duration::from_millis(50));
            if let Some(subscription) = subscription {
                let _ = subscription.receiver().recv_timeout(pause);
            } else {
                std::thread::sleep(pause);
            }
            if std::time::Instant::now() >= deadline {
                return Ok((after, observations));
            }
        }
    }
}

fn remaining(deadline: std::time::Instant, timeout_ms: u64) -> WarResult<std::time::Duration> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| WarError::Timeout {
            operation: "action batch".into(),
            timeout_ms,
        })
}

fn uses_unscoped_input(action: &war_protocol::Action) -> bool {
    matches!(
        action,
        war_protocol::Action::TypeText { .. }
            | war_protocol::Action::Key { .. }
            | war_protocol::Action::Click {
                target: Target::Coordinates(_),
                ..
            }
    )
}

fn ensure_foreground_process(target_process: u32) -> WarResult<()> {
    let foreground_process = war_win32::foreground_process_id();
    if target_process != 0 && target_process == foreground_process {
        Ok(())
    } else {
        Err(WarError::ForegroundMismatch {
            target_process,
            foreground_process,
        })
    }
}

fn ensure_capability(
    snapshot: &war_protocol::SemanticSnapshot,
    id: war_protocol::NodeId,
    action: &war_protocol::Action,
) -> WarResult<()> {
    let Some(required) = required_capability_flag(action) else {
        return Ok(());
    };
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or_else(|| WarError::TargetNotFound(format!("@{id}")))?;
    if node.states.enabled && node.capabilities.contains(required) {
        return Ok(());
    }
    Err(WarError::CapabilityUnavailable(format!(
        "@{id} does not support {}",
        required_capability(action).unwrap_or("requested action")
    )))
}
