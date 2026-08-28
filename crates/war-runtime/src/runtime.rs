use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use war_core::{DesktopProvider, LocalVerifier, Session};
use war_semantic::SemanticCompiler;

pub struct WarRuntime {
    pub(crate) provider: Arc<dyn DesktopProvider>,
    pub(crate) compiler: Mutex<SemanticCompiler>,
    pub(crate) session: Mutex<Session>,
    pub(crate) verifier: LocalVerifier,
}

impl WarRuntime {
    pub fn new(provider: Arc<dyn DesktopProvider>) -> Self {
        Self {
            provider,
            compiler: Mutex::new(SemanticCompiler::new()),
            session: Mutex::new(Session::new(new_session_id())),
            verifier: LocalVerifier,
        }
    }
}

fn new_session_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}-{:x}", std::process::id(), time, sequence)
}
