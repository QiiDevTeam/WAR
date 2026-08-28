use crate::worker::{spawn_worker, UiaCommand};
use crossbeam_channel::{bounded, unbounded, RecvTimeoutError, Sender};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use war_core::{DesktopProvider, ProviderActionResult, ProviderNodeRef, Subscription};
use war_protocol::{
    Action, DesktopEvent, NodeSource, RawSnapshot, SnapshotScope, WarError, WarResult,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ABANDONED_WORKERS: usize = 3;

pub struct UiaProvider {
    worker: Mutex<Sender<UiaCommand>>,
    subscribers: Mutex<Vec<Sender<DesktopEvent>>>,
    timeout: Duration,
    worker_factory: fn() -> WarResult<Sender<UiaCommand>>,
    abandoned_workers: AtomicUsize,
    circuit_open: AtomicBool,
}

impl UiaProvider {
    pub fn new() -> WarResult<Self> {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(timeout: Duration) -> WarResult<Self> {
        Self::with_worker_factory(timeout, spawn_worker)
    }

    fn with_worker_factory(
        timeout: Duration,
        worker_factory: fn() -> WarResult<Sender<UiaCommand>>,
    ) -> WarResult<Self> {
        if timeout.is_zero() {
            return Err(WarError::InvalidRequest(
                "UIA request timeout must be greater than zero".into(),
            ));
        }
        Ok(Self {
            worker: Mutex::new(worker_factory()?),
            subscribers: Mutex::new(Vec::new()),
            timeout,
            worker_factory,
            abandoned_workers: AtomicUsize::new(0),
            circuit_open: AtomicBool::new(false),
        })
    }

    fn restart(&self, worker: &mut Sender<UiaCommand>) -> WarResult<()> {
        *worker = (self.worker_factory)()?;
        let subscribers = self
            .subscribers
            .lock()
            .map_err(|_| WarError::Provider("UIA subscriber lock was poisoned".into()))?;
        for subscriber in subscribers.iter() {
            worker
                .send(UiaCommand::Subscribe(subscriber.clone()))
                .map_err(|error| WarError::Provider(error.to_string()))?;
        }
        Ok(())
    }

    fn timed_out(operation: &str, timeout: Duration) -> WarError {
        WarError::Timeout {
            operation: operation.into(),
            timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
        }
    }

    fn ensure_available(&self) -> WarResult<()> {
        if self.circuit_open.load(Ordering::Acquire) {
            Err(WarError::Provider(
                "UIA timeout circuit is open; recreate the provider process".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn restart_after_timeout(&self, worker: &mut Sender<UiaCommand>) -> WarResult<()> {
        let abandoned = self.abandoned_workers.fetch_add(1, Ordering::AcqRel) + 1;
        if abandoned > MAX_ABANDONED_WORKERS {
            self.circuit_open.store(true, Ordering::Release);
            return Err(WarError::Provider(format!(
                "UIA timeout circuit opened after {MAX_ABANDONED_WORKERS} worker replacements"
            )));
        }
        self.restart(worker)
    }

    fn snapshot_deadline(&self, scope: SnapshotScope, timeout: Duration) -> WarResult<RawSnapshot> {
        self.ensure_available()?;
        let timeout = timeout.min(self.timeout);
        let (reply, response) = bounded(1);
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| WarError::Provider("UIA worker lock was poisoned".into()))?;
        if worker.send(UiaCommand::Snapshot(scope, reply)).is_err() {
            self.restart(&mut worker)?;
            return Err(WarError::Provider(
                "UIA worker disconnected; restarted".into(),
            ));
        }
        match response.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.restart_after_timeout(&mut worker)?;
                Err(Self::timed_out("UIA snapshot", timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.restart(&mut worker)?;
                Err(WarError::Provider(
                    "UIA worker disconnected; restarted".into(),
                ))
            }
        }
    }

    fn execute_deadline(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
        timeout: Duration,
    ) -> WarResult<ProviderActionResult> {
        self.ensure_available()?;
        let timeout = timeout.min(self.timeout);
        let (reply, response) = bounded(1);
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| WarError::Provider("UIA worker lock was poisoned".into()))?;
        if worker
            .send(UiaCommand::Execute(node, action.clone(), reply))
            .is_err()
        {
            self.restart(&mut worker)?;
            return Err(WarError::Provider(
                "UIA worker disconnected; restarted".into(),
            ));
        }
        match response.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.restart_after_timeout(&mut worker)?;
                Err(Self::timed_out("UIA action", timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.restart(&mut worker)?;
                Err(WarError::Provider(
                    "UIA worker disconnected; restarted".into(),
                ))
            }
        }
    }
}

impl Drop for UiaProvider {
    fn drop(&mut self) {
        if let Ok(worker) = self.worker.get_mut() {
            let _ = worker.send(UiaCommand::Shutdown);
        }
    }
}

impl DesktopProvider for UiaProvider {
    fn kind(&self) -> NodeSource {
        NodeSource::Uia
    }

    fn snapshot(&self, scope: SnapshotScope) -> WarResult<RawSnapshot> {
        self.snapshot_deadline(scope, self.timeout)
    }

    fn snapshot_with_timeout(
        &self,
        scope: SnapshotScope,
        timeout: Duration,
    ) -> WarResult<RawSnapshot> {
        self.snapshot_deadline(scope, timeout)
    }

    fn execute(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
    ) -> WarResult<ProviderActionResult> {
        self.execute_deadline(node, action, self.timeout)
    }

    fn execute_with_timeout(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
        timeout: Duration,
    ) -> WarResult<ProviderActionResult> {
        self.execute_deadline(node, action, timeout)
    }

    fn subscribe(&self) -> WarResult<Subscription> {
        let (sink, receiver) = unbounded::<DesktopEvent>();
        let worker = self
            .worker
            .lock()
            .map_err(|_| WarError::Provider("UIA worker lock was poisoned".into()))?;
        worker
            .send(UiaCommand::Subscribe(sink.clone()))
            .map_err(|error| WarError::Provider(error.to_string()))?;
        self.subscribers
            .lock()
            .map_err(|_| WarError::Provider("UIA subscriber lock was poisoned".into()))?
            .push(sink);
        Ok(Subscription::new(receiver))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    static SPAWNS: AtomicUsize = AtomicUsize::new(0);

    fn controlled_worker() -> WarResult<Sender<UiaCommand>> {
        let generation = SPAWNS.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = unbounded();
        thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                match command {
                    UiaCommand::Snapshot(_, reply) => {
                        if generation == 0 {
                            thread::sleep(Duration::from_millis(100));
                        }
                        let _ = reply.send(Ok(RawSnapshot {
                            epoch: generation as u64 + 1,
                            root: 0,
                            nodes: HashMap::new(),
                        }));
                    }
                    UiaCommand::Shutdown => break,
                    UiaCommand::Execute(_, _, reply) => {
                        let _ = reply.send(Err(WarError::Provider("not used".into())));
                    }
                    UiaCommand::Subscribe(sink) => {
                        if generation > 0 {
                            let _ = sink.send(DesktopEvent::PropertyChanged {
                                node: 0,
                                property: war_protocol::Property::State,
                            });
                        }
                    }
                }
            }
        });
        Ok(sender)
    }

    fn always_hung_worker() -> WarResult<Sender<UiaCommand>> {
        let (sender, receiver) = unbounded();
        thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                match command {
                    UiaCommand::Snapshot(_, reply) => {
                        thread::sleep(Duration::from_millis(100));
                        let _ = reply.send(Err(WarError::Provider("late".into())));
                    }
                    UiaCommand::Shutdown => break,
                    UiaCommand::Execute(_, _, reply) => {
                        thread::sleep(Duration::from_millis(100));
                        let _ = reply.send(Err(WarError::Provider("late".into())));
                    }
                    UiaCommand::Subscribe(_) => {}
                }
            }
        });
        Ok(sender)
    }

    #[test]
    fn timeout_restarts_worker_and_next_request_succeeds() {
        SPAWNS.store(0, Ordering::SeqCst);
        let provider =
            UiaProvider::with_worker_factory(Duration::from_millis(20), controlled_worker).unwrap();
        let subscription = provider.subscribe().unwrap();
        assert!(matches!(
            provider.snapshot(SnapshotScope::Desktop),
            Err(WarError::Timeout { .. })
        ));
        assert_eq!(provider.snapshot(SnapshotScope::Desktop).unwrap().epoch, 2);
        assert!(matches!(
            subscription
                .receiver()
                .recv_timeout(Duration::from_millis(20)),
            Ok(DesktopEvent::PropertyChanged { .. })
        ));
    }

    #[test]
    fn repeated_timeouts_open_a_bounded_circuit() {
        let provider =
            UiaProvider::with_worker_factory(Duration::from_millis(5), always_hung_worker).unwrap();
        for _ in 0..MAX_ABANDONED_WORKERS {
            assert!(matches!(
                provider.snapshot(SnapshotScope::Desktop),
                Err(WarError::Timeout { .. })
            ));
        }
        assert!(matches!(
            provider.snapshot(SnapshotScope::Desktop),
            Err(WarError::Provider(message)) if message.contains("circuit opened")
        ));
        let started = std::time::Instant::now();
        assert!(matches!(
            provider.snapshot(SnapshotScope::Desktop),
            Err(WarError::Provider(message)) if message.contains("circuit is open")
        ));
        assert!(started.elapsed() < Duration::from_millis(5));
    }

    #[test]
    fn caller_deadline_is_stricter_than_provider_default() {
        let provider =
            UiaProvider::with_worker_factory(Duration::from_millis(50), always_hung_worker)
                .unwrap();
        assert!(matches!(
            provider.snapshot_with_timeout(SnapshotScope::Desktop, Duration::from_millis(7)),
            Err(WarError::Timeout { timeout_ms: 7, .. })
        ));
    }
}
