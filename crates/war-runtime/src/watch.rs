use crate::WarRuntime;
use std::time::{Duration, Instant};
use war_protocol::{SnapshotDelta, SnapshotScope, WarResult};
use war_semantic::diff;

impl WarRuntime {
    pub fn watch<F>(&self, scope: SnapshotScope, mut emit: F) -> WarResult<()>
    where
        F: FnMut(SnapshotDelta) -> bool,
    {
        let subscription = self.provider.subscribe()?;
        let mut before = self.observe(scope.clone())?;
        loop {
            if subscription.receiver().recv().is_err() {
                return Ok(());
            }
            let deadline = Instant::now() + Duration::from_millis(25);
            while Instant::now() < deadline {
                if subscription
                    .receiver()
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .is_err()
                {
                    break;
                }
            }
            let after = self.observe(scope.clone())?;
            let delta = diff(&before, &after);
            let changed = !delta.added.is_empty()
                || !delta.removed.is_empty()
                || !delta.changed.is_empty()
                || delta.focus_changed.is_some();
            before = after;
            if changed && !emit(delta) {
                return Ok(());
            }
        }
    }
}
