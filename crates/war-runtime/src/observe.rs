use crate::WarRuntime;
use war_core::SessionSnapshot;
use war_protocol::{SemanticSnapshot, SnapshotScope, WarError, WarResult, WindowInfo};

impl WarRuntime {
    pub fn observe(&self, scope: SnapshotScope) -> WarResult<SemanticSnapshot> {
        self.observe_inner(scope, None)
    }

    pub(crate) fn observe_with_timeout(
        &self,
        scope: SnapshotScope,
        timeout: std::time::Duration,
    ) -> WarResult<SemanticSnapshot> {
        self.observe_inner(scope, Some(timeout))
    }

    fn observe_inner(
        &self,
        scope: SnapshotScope,
        timeout: Option<std::time::Duration>,
    ) -> WarResult<SemanticSnapshot> {
        let provider_scope = match scope {
            SnapshotScope::Node(stable) => {
                let session = self.session.lock().map_err(poisoned)?;
                let current = session.current.as_ref().ok_or_else(|| {
                    WarError::InvalidRequest(
                        "node scope requires a snapshot from the same session".into(),
                    )
                })?;
                let provider = current.provider_refs.get(&stable).ok_or_else(|| {
                    WarError::TargetNotFound(format!("provider mapping for @{stable}"))
                })?;
                SnapshotScope::Node(provider.id)
            }
            _ => scope.clone(),
        };
        let raw = match timeout {
            Some(timeout) => self
                .provider
                .snapshot_with_timeout(provider_scope, timeout)?,
            None => self.provider.snapshot(provider_scope)?,
        };
        let root = raw.nodes.get(&raw.root);
        let process_id = root
            .map(|node| node.fingerprint.process_id)
            .unwrap_or_default();
        let mut window = match &scope {
            SnapshotScope::Desktop => WindowInfo::default(),
            SnapshotScope::Window(id) => war_win32::window_info(war_win32::id_to_hwnd(*id)),
            SnapshotScope::Process(id) => war_win32::process_window(*id)
                .map(war_win32::window_info)
                .unwrap_or_default(),
            SnapshotScope::FocusedWindow => war_win32::foreground_window()
                .map(war_win32::window_info)
                .unwrap_or_default(),
            SnapshotScope::Node(_) | SnapshotScope::FocusedSubtree => {
                war_win32::process_window(process_id)
                    .map(war_win32::window_info)
                    .unwrap_or_default()
            }
        };
        if window.process_id == 0 {
            window.process_id = process_id;
            window.app = war_win32::process_name(process_id);
        }
        if window.title.is_none() {
            window.title = root.and_then(|node| node.name.clone());
        }
        let scope_changed = {
            let session = self.session.lock().map_err(poisoned)?;
            session.current.as_ref().is_some_and(|current| {
                identity_changed(&current.scope, &scope, &session.window, &window)
            })
        };
        let mut compiler = self.compiler.lock().map_err(poisoned)?;
        if scope_changed {
            compiler.reset_history();
        }
        let mut compiled = compiler.compile(raw, window.clone());
        drop(compiler);
        let session_id = self.session.lock().map_err(poisoned)?.id.clone();
        compiled.semantic.session_id = session_id;
        let semantic = compiled.semantic.clone();
        let mut session = self.session.lock().map_err(poisoned)?;
        session.window = window;
        session.current = Some(SessionSnapshot {
            semantic: compiled.semantic,
            provider_refs: compiled.provider_refs,
            bounds: compiled.bounds,
            scope,
        });
        Ok(semantic)
    }

    pub fn current(&self) -> WarResult<Option<SemanticSnapshot>> {
        Ok(self
            .session
            .lock()
            .map_err(poisoned)?
            .current
            .as_ref()
            .map(|current| current.semantic.clone()))
    }

    pub(crate) fn current_bounds(
        &self,
        id: war_protocol::NodeId,
    ) -> WarResult<Option<war_protocol::Rect>> {
        Ok(self
            .session
            .lock()
            .map_err(poisoned)?
            .current
            .as_ref()
            .and_then(|current| current.bounds.get(&id).copied()))
    }
}

fn identity_changed(
    previous_scope: &SnapshotScope,
    next_scope: &SnapshotScope,
    previous_window: &WindowInfo,
    next_window: &WindowInfo,
) -> bool {
    if previous_window.process_id != next_window.process_id {
        return true;
    }
    match (previous_scope, next_scope) {
        (SnapshotScope::Desktop, SnapshotScope::Desktop) => false,
        (SnapshotScope::Process(before), SnapshotScope::Process(after)) => before != after,
        _ => previous_window.id != next_window.id,
    }
}

pub(crate) fn poisoned<T>(_: std::sync::PoisonError<T>) -> WarError {
    WarError::Provider("runtime state lock was poisoned".into())
}
