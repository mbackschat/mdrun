use super::custody::{Custody, CustodyEvent, GroupEvent, GroupKey, OwnerRequest, SpawnSpec};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(5);
const EVENT_CAPACITY: usize = 16;

pub enum CustodyAccess {
    Standalone(Custody),
    Scoped(ScopedCustody),
}

impl CustodyAccess {
    pub fn spawn(
        &mut self,
        id: &str,
        spec: &SpawnSpec,
        stdio: [BorrowedFd<'_>; 3],
    ) -> io::Result<CustodyEvent> {
        match self {
            Self::Standalone(custody) => custody.spawn(id, spec, stdio),
            Self::Scoped(custody) => custody.spawn(id, spec, stdio),
        }
    }

    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<CustodyEvent>> {
        match self {
            Self::Standalone(custody) => custody.poll_event(timeout),
            Self::Scoped(custody) => custody.poll_event(timeout),
        }
    }

    pub fn stop(&mut self) -> io::Result<()> {
        match self {
            Self::Standalone(custody) => custody.stop(),
            Self::Scoped(custody) => {
                custody.signals.stop.store(true, Ordering::Release);
                Ok(())
            }
        }
    }

    pub fn pid(&self) -> u32 {
        match self {
            Self::Standalone(custody) => custody.pid(),
            Self::Scoped(custody) => custody.pid,
        }
    }

    pub fn status_path(&self) -> &Path {
        match self {
            Self::Standalone(custody) => custody.status_path(),
            Self::Scoped(custody) => &custody.status_path,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BrokerStatus {
    pub stopped: bool,
    pub failed: bool,
    pub uncertain: bool,
    pub finished: bool,
}

#[derive(Default)]
struct Signals {
    stop: AtomicBool,
    lost: AtomicBool,
}

enum Request {
    Register {
        scope: String,
        deadline: Instant,
        cleanup: Duration,
        events: SyncSender<CustodyEvent>,
        signals: Arc<Signals>,
    },
    Spawn {
        key: GroupKey,
        spec: SpawnSpec,
        stdio: [OwnedFd; 3],
    },
}

pub struct CustodyBroker {
    requests: SyncSender<Request>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<BrokerStatus>>,
    pid: u32,
    status_path: PathBuf,
    thread: Option<thread::JoinHandle<()>>,
}

impl CustodyBroker {
    pub fn start(custody: Custody, capacity: usize) -> io::Result<Self> {
        if capacity == 0 {
            return Err(io::Error::other("broker capacity must be positive"));
        }
        let (requests, receiver) = mpsc::sync_channel(capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(BrokerStatus::default()));
        let pid = custody.pid();
        let status_path = custody.status_path().to_owned();
        let thread_stop = stop.clone();
        let thread_status = status.clone();
        let thread = thread::Builder::new()
            .name("mdrun-custody-broker".to_owned())
            .spawn(move || {
                serve(custody, receiver, &thread_stop, &thread_status);
            })?;
        Ok(Self {
            requests,
            stop,
            status,
            pid,
            status_path,
            thread: Some(thread),
        })
    }

    pub fn register(
        &self,
        scope: String,
        deadline: Instant,
        cleanup: Duration,
    ) -> io::Result<ScopedCustody> {
        self.register_with_capacity(scope, deadline, cleanup, EVENT_CAPACITY)
    }

    fn register_with_capacity(
        &self,
        scope: String,
        deadline: Instant,
        cleanup: Duration,
        capacity: usize,
    ) -> io::Result<ScopedCustody> {
        let (events, receiver) = mpsc::sync_channel(capacity);
        let signals = Arc::new(Signals::default());
        let handle = ScopedCustody {
            scope: scope.clone(),
            requests: self.requests.clone(),
            events: receiver,
            signals: signals.clone(),
            batch_stop: self.stop.clone(),
            status: self.status.clone(),
            deadline,
            pid: self.pid,
            status_path: self.status_path.clone(),
            pending: VecDeque::new(),
        };
        let request = Request::Register {
            scope: scope.clone(),
            deadline,
            cleanup,
            events,
            signals,
        };
        handle.send(request)?;
        match handle.events.recv_timeout(remaining(deadline)?) {
            Ok(CustodyEvent::ScopeRegistered { scope: registered }) if registered == scope => {
                Ok(handle)
            }
            _ => Err(io::Error::other("custody scope registration failed")),
        }
    }

    pub fn status(&self) -> BrokerStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn finish(mut self, deadline: Instant) -> io::Result<BrokerStatus> {
        self.stop.store(true, Ordering::Release);
        while !self.status().finished {
            remaining(deadline)?;
            thread::sleep(TICK);
        }
        if self.thread.take().unwrap().join().is_err() {
            return Err(io::Error::other("custody broker panicked"));
        }
        Ok(self.status())
    }
}

#[cfg(test)]
mod tests {
    use super::super::custody::CustodyConfig;
    use super::*;
    use std::fs::{self, File};
    use std::net::{SocketAddr, TcpListener};

    struct Fixture {
        broker: Option<CustodyBroker>,
        directory: PathBuf,
        address: SocketAddr,
    }

    impl Fixture {
        fn new() -> Self {
            let executable = PathBuf::from(
                std::env::var_os("MDRUN_NATIVE_TEST_BIN")
                    .expect("set MDRUN_NATIVE_TEST_BIN to the built native binary"),
            );
            assert!(executable.is_absolute() && executable.is_file());
            let directory =
                std::env::temp_dir().join(format!("mdrun-broker-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&directory).unwrap();
            let address = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap();
            let config = CustodyConfig {
                resources: vec![address],
                session_timeout_ms: 20000,
                cleanup_timeout_ms: 1000,
            };
            let custody =
                Custody::start(&executable, &directory.join("custody.json"), &config).unwrap();
            Self {
                broker: Some(CustodyBroker::start(custody, 1).unwrap()),
                directory,
                address,
            }
        }

        fn scope(&self, id: &str) -> CustodyAccess {
            CustodyAccess::Scoped(
                self.broker
                    .as_ref()
                    .unwrap()
                    .register(
                        id.to_owned(),
                        Instant::now() + Duration::from_secs(10),
                        Duration::from_millis(1000),
                    )
                    .unwrap(),
            )
        }

        fn excluded(&self) -> bool {
            match TcpListener::bind(self.address) {
                Ok(_) => false,
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
                    true
                }
            }
        }

        fn finish(&mut self) -> BrokerStatus {
            self.broker
                .take()
                .unwrap()
                .finish(Instant::now() + Duration::from_secs(5))
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let clean = self
                .broker
                .take()
                .map(|broker| {
                    broker
                        .finish(Instant::now() + Duration::from_secs(5))
                        .is_ok_and(|state| state.stopped)
                })
                .unwrap_or_else(|| !self.excluded());
            if clean {
                let _ = fs::remove_dir_all(&self.directory);
            } else {
                eprintln!(
                    "broker fixture retained custody evidence: {}",
                    self.directory.display()
                );
            }
        }
    }

    fn launch(fixture: &Fixture, custody: &mut CustodyAccess, name: &str, command: &str) {
        let input = File::open("/dev/null").unwrap();
        let output = File::create(fixture.directory.join(name)).unwrap();
        let spec = SpawnSpec {
            argv: vec!["/bin/sh".into(), "-c".into(), command.into()],
            cwd: PathBuf::from("/"),
            environment: vec![],
            umask: 0o022,
            caller_group: unsafe { libc::getpgrp() },
        };
        assert!(
            matches!(custody.spawn("author", &spec, [input.as_fd(),output.as_fd(),output.as_fd()]).unwrap(), CustodyEvent::Started { id, .. } if id=="author")
        );
    }

    fn complete(custody: &mut CustodyAccess, code: i32) {
        assert_eq!(
            custody.poll_event(Duration::from_secs(3)).unwrap(),
            Some(CustodyEvent::Exited {
                id: "author".to_owned(),
                status: super::super::custody::PayloadStatus {
                    code: Some(code),
                    signal: None
                }
            })
        );
        assert_eq!(
            custody.poll_event(Duration::from_secs(3)).unwrap(),
            Some(CustodyEvent::Clean {
                id: "author".to_owned(),
                forced: false
            })
        );
        custody.stop().unwrap();
        assert_eq!(
            custody.poll_event(Duration::from_secs(3)).unwrap(),
            Some(CustodyEvent::Stopped { clean: true })
        );
    }

    #[test]
    #[ignore = "requires MDRUN_NATIVE_TEST_BIN pointing to the separately built native binary"]
    fn native_scopes_interleave_without_crossing_descriptors_or_releasing_the_batch_lease() {
        let mut fixture = Fixture::new();
        let mut first = fixture.scope("first");
        let mut second = fixture.scope("second");
        launch(
            &fixture,
            &mut first,
            "first.out",
            "printf first; printf error-first >&2; exit 7",
        );
        launch(&fixture, &mut second, "second.out", "printf second; exit 9");
        complete(&mut second, 9);
        complete(&mut first, 7);
        assert_eq!(
            fs::read(fixture.directory.join("first.out")).unwrap(),
            b"firsterror-first"
        );
        assert_eq!(
            fs::read(fixture.directory.join("second.out")).unwrap(),
            b"second"
        );
        assert!(fixture.excluded());
        let result = fixture.finish();
        assert!(
            result.stopped && !result.failed && !result.uncertain,
            "{result:?}"
        );
        assert!(!fixture.excluded());
    }

    #[test]
    #[ignore = "requires MDRUN_NATIVE_TEST_BIN pointing to the separately built native binary"]
    fn native_coordinator_loss_stops_only_its_scope_and_preserves_authoritative_cleanup() {
        let mut fixture = Fixture::new();
        let mut lost = fixture.scope("lost");
        let mut healthy = fixture.scope("healthy");
        launch(&fixture, &mut lost, "lost.out", "exec /bin/sleep 15");
        drop(lost);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state: serde_json::Value = serde_json::from_slice(
                &fs::read(fixture.directory.join("custody.status.json")).unwrap(),
            )
            .unwrap();
            if state["scopes"][0]["phase"] == "stopped" {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(TICK);
        }
        assert!(fixture.excluded());
        launch(&fixture, &mut healthy, "healthy.out", "printf survived");
        complete(&mut healthy, 0);
        assert_eq!(
            fs::read(fixture.directory.join("healthy.out")).unwrap(),
            b"survived"
        );
        let result = fixture.finish();
        assert!(
            result.stopped && result.failed && !result.uncertain,
            "{result:?}"
        );
        assert!(!fixture.excluded());
    }

    #[test]
    #[ignore = "requires MDRUN_NATIVE_TEST_BIN pointing to the separately built native binary"]
    fn native_event_overflow_stops_the_batch_without_blocking_cleanup() {
        let mut fixture = Fixture::new();
        let mut scope = CustodyAccess::Scoped(
            fixture
                .broker
                .as_ref()
                .unwrap()
                .register_with_capacity(
                    "slow".to_owned(),
                    Instant::now() + Duration::from_secs(10),
                    Duration::from_millis(1000),
                    1,
                )
                .unwrap(),
        );
        launch(&fixture, &mut scope, "slow.out", "printf done");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.broker.as_ref().unwrap().status().finished {
            assert!(Instant::now() < deadline);
            thread::sleep(TICK);
        }
        let result = fixture.finish();
        assert!(
            result.stopped && result.failed && !result.uncertain,
            "{result:?}"
        );
        assert!(!fixture.excluded());
    }

    #[test]
    fn stop_and_coordinator_loss_do_not_depend_on_ordinary_queue_capacity() {
        let (requests, receiver) = mpsc::sync_channel(1);
        let (events, incoming) = mpsc::sync_channel(1);
        let signals = Arc::new(Signals::default());
        requests
            .try_send(Request::Register {
                scope: "queued".to_owned(),
                deadline: Instant::now() + Duration::from_secs(1),
                cleanup: Duration::from_secs(1),
                events,
                signals: signals.clone(),
            })
            .unwrap_or_else(|_| panic!("empty queue rejected request"));
        let mut custody = CustodyAccess::Scoped(ScopedCustody {
            scope: "queued".to_owned(),
            requests,
            events: incoming,
            signals: signals.clone(),
            batch_stop: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(BrokerStatus::default())),
            deadline: Instant::now() + Duration::from_secs(1),
            pid: 0,
            status_path: PathBuf::new(),
            pending: VecDeque::new(),
        });
        custody.stop().unwrap();
        assert!(signals.stop.load(Ordering::Acquire));
        drop(custody);
        assert!(signals.lost.load(Ordering::Acquire));
        assert!(matches!(receiver.try_recv(), Ok(Request::Register { .. })));
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn repeated_rejected_spawns_cannot_accumulate_unbounded_pending_faults() {
        let (requests, receiver) = mpsc::sync_channel(1);
        let (events, incoming) = mpsc::sync_channel(1);
        let batch_stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(BrokerStatus::default()));
        let mut custody = ScopedCustody {
            scope: "closed".to_owned(),
            requests,
            events: incoming,
            signals: Arc::new(Signals::default()),
            batch_stop: batch_stop.clone(),
            status: status.clone(),
            deadline: Instant::now() + Duration::from_secs(5),
            pid: 0,
            status_path: PathBuf::new(),
            pending: VecDeque::new(),
        };
        let responder = thread::spawn(move || {
            while let Ok(Request::Spawn { key, .. }) = receiver.recv() {
                if events
                    .send(CustodyEvent::ScopeEvent {
                        scope: "closed".to_owned(),
                        event: GroupEvent::Rejected {
                            id: key.id,
                            reason: "scope admission closed".to_owned(),
                        },
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let descriptor = File::open("/dev/null").unwrap();
        let spec = SpawnSpec {
            argv: vec!["/unused".into()],
            cwd: PathBuf::from("/"),
            environment: vec![],
            umask: 0o022,
            caller_group: 1,
        };
        for index in 0..=EVENT_CAPACITY {
            assert!(
                custody
                    .spawn(&index.to_string(), &spec, [descriptor.as_fd(); 3])
                    .is_err()
            );
        }
        let count = custody.pending.len();
        drop(custody);
        responder.join().unwrap();
        assert!(
            count <= EVENT_CAPACITY,
            "pending faults exceeded capacity: {count}"
        );
        assert!(batch_stop.load(Ordering::Acquire));
        assert!(status.lock().unwrap().failed);
    }

    fn entry(phase: ScopePhase) -> (Entry, Receiver<CustodyEvent>) {
        let (events, receiver) = mpsc::sync_channel(4);
        (
            Entry {
                events,
                signals: Arc::new(Signals::default()),
                phase,
                groups: BTreeSet::from(["author".to_owned()]),
            },
            receiver,
        )
    }

    #[test]
    fn rejected_inflight_registration_does_not_abort_an_admitted_sibling() {
        let (failed, _failed_events) = entry(ScopePhase::Running);
        let (healthy, healthy_events) = entry(ScopePhase::Running);
        let (queued, queued_events) = entry(ScopePhase::Registering);
        let mut scopes = BTreeMap::from([
            ("failed".to_owned(), failed),
            ("healthy".to_owned(), healthy),
            ("queued".to_owned(), queued),
        ]);
        let status = Mutex::new(BrokerStatus::default());
        let stop = AtomicBool::new(false);
        dispatch_event(
            CustodyEvent::ScopeEvent {
                scope: "failed".to_owned(),
                event: GroupEvent::Fault {
                    reason: "scope deadline expired".to_owned(),
                },
            },
            &mut scopes,
            &status,
            &stop,
        );
        let rejection = CustodyEvent::ScopeRejected {
            scope: "queued".to_owned(),
            reason: "scope admission closed".to_owned(),
        };
        dispatch_event(rejection.clone(), &mut scopes, &status, &stop);
        assert_eq!(queued_events.try_recv().unwrap(), rejection);
        assert_eq!(scopes["queued"].phase, ScopePhase::Rejected);
        assert!(scopes["queued"].phase.terminal());
        assert!(!stop.load(Ordering::Acquire));
        let started = CustodyEvent::ScopeEvent {
            scope: "healthy".to_owned(),
            event: GroupEvent::Started {
                id: "author".to_owned(),
                pid: 41,
                group: 40,
            },
        };
        dispatch_event(started.clone(), &mut scopes, &status, &stop);
        assert_eq!(healthy_events.try_recv().unwrap(), started);
        assert_eq!(scopes["healthy"].phase, ScopePhase::Running);
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn impossible_registration_completion_or_group_attribution_stops_the_batch() {
        for (phase, event) in [
            (
                ScopePhase::Registering,
                CustodyEvent::ScopeStopped {
                    scope: "document".to_owned(),
                    clean: true,
                },
            ),
            (
                ScopePhase::Rejected,
                CustodyEvent::ScopeRegistered {
                    scope: "document".to_owned(),
                },
            ),
            (
                ScopePhase::Stopped,
                CustodyEvent::ScopeStopped {
                    scope: "document".to_owned(),
                    clean: true,
                },
            ),
            (
                ScopePhase::Running,
                CustodyEvent::ScopeEvent {
                    scope: "document".to_owned(),
                    event: GroupEvent::Clean {
                        id: "unknown".to_owned(),
                        forced: false,
                    },
                },
            ),
        ] {
            let (entry, _events) = entry(phase);
            let mut scopes = BTreeMap::from([("document".to_owned(), entry)]);
            let status = Mutex::new(BrokerStatus::default());
            let stop = AtomicBool::new(false);
            assert!(!dispatch_event(event, &mut scopes, &status, &stop));
            assert!(stop.load(Ordering::Acquire));
            assert!(status.lock().unwrap().failed);
            assert_eq!(scopes["document"].phase, phase);
        }
    }

    #[test]
    #[ignore = "requires MDRUN_NATIVE_TEST_BIN pointing to the separately built native binary"]
    fn native_prepared_hosts_use_owned_stdio_and_return_document_reports() {
        use super::super::config::Binding;
        use super::super::context::NativeContext;
        use super::super::host::{AuthorStdio, PreparedSession, run_prepared};
        use super::super::{Mode, Options};
        let mut fixture = Fixture::new();
        let mut tasks = Vec::new();
        for (name, code) in [("first", 7), ("second", 0)] {
            let CustodyAccess::Scoped(scope) = fixture.scope(name) else {
                unreachable!()
            };
            let binding: Binding = serde_json::from_value(serde_json::json!({"version":1,"name":"tool","artifact_id":"test","worker_argv":["/unused"],"cold_argv":["/bin/echo"],"session_timeout_ms":10000,"cleanup_timeout_ms":1000})).unwrap();
            let baseline = NativeContext {
                cwd: fixture.directory.clone(),
                environment: vec![("PATH".into(), "/usr/bin:/bin".into())],
                umask: 0o027,
                caller_group: unsafe { libc::getpgrp() },
            };
            let options = Options {
                binding: PathBuf::new(),
                mode: Mode::Cold,
                report: None,
                require_warm: false,
            };
            let prepared = PreparedSession::from_context(
                options,
                binding,
                vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("printf {name}; printf error-{name} >&2; exit {code}").into(),
                ],
                baseline,
                None,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            let stdio = AuthorStdio([
                File::open("/dev/null").unwrap().into(),
                File::create(fixture.directory.join(format!("{name}.stdout")))
                    .unwrap()
                    .into(),
                File::create(fixture.directory.join(format!("{name}.stderr")))
                    .unwrap()
                    .into(),
            ]);
            tasks.push((
                name,
                code,
                thread::spawn(move || run_prepared(prepared, stdio, Some(scope)).unwrap()),
            ));
        }
        let pid = fixture.broker.as_ref().unwrap().pid;
        for (name, code, task) in tasks {
            let outcome = task.join().unwrap();
            assert_eq!(outcome.status, code);
            assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
            assert_eq!(outcome.report["version"], 1);
            assert_eq!(
                outcome.report["author_status"],
                serde_json::json!({"exit_code":code,"signal":null})
            );
            assert_eq!(outcome.report["cleanup"]["outcome"], "clean");
            assert_eq!(outcome.report["cleanup"]["custodian_pid"], pid);
            assert_eq!(
                outcome.report["infrastructure_failure"],
                serde_json::Value::Null
            );
            assert_eq!(outcome.report["worker_starts"], 0);
            assert_eq!(
                fs::read(fixture.directory.join(format!("{name}.stdout"))).unwrap(),
                name.as_bytes()
            );
            assert_eq!(
                fs::read(fixture.directory.join(format!("{name}.stderr"))).unwrap(),
                format!("error-{name}").as_bytes()
            );
        }
        assert!(fixture.excluded());
        assert!(fixture.finish().stopped);
        assert!(!fixture.excluded());
    }
}

impl Drop for CustodyBroker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub struct ScopedCustody {
    scope: String,
    requests: SyncSender<Request>,
    events: Receiver<CustodyEvent>,
    signals: Arc<Signals>,
    batch_stop: Arc<AtomicBool>,
    status: Arc<Mutex<BrokerStatus>>,
    deadline: Instant,
    pid: u32,
    status_path: PathBuf,
    pending: VecDeque<CustodyEvent>,
}

impl ScopedCustody {
    fn send(&self, mut request: Request) -> io::Result<()> {
        loop {
            remaining(self.deadline)?;
            if self.batch_stop.load(Ordering::Acquire) || self.status.lock().unwrap().finished {
                return Err(io::Error::other("custody broker stopped"));
            }
            match self.requests.try_send(request) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    request = returned;
                    thread::sleep(TICK);
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(io::Error::other("custody broker lost"));
                }
            }
        }
    }

    fn spawn(
        &mut self,
        id: &str,
        spec: &SpawnSpec,
        stdio: [BorrowedFd<'_>; 3],
    ) -> io::Result<CustodyEvent> {
        let stdio = [
            stdio[0].try_clone_to_owned()?,
            stdio[1].try_clone_to_owned()?,
            stdio[2].try_clone_to_owned()?,
        ];
        self.send(Request::Spawn {
            key: GroupKey {
                scope: Some(self.scope.clone()),
                id: id.to_owned(),
            },
            spec: spec.clone(),
            stdio,
        })?;
        loop {
            let event = self
                .receive(remaining(self.deadline)?)?
                .ok_or_else(|| io::Error::other("scope launch deadline expired"))?;
            match &event {
                CustodyEvent::Started { id: started, .. } if started == id => return Ok(event),
                CustodyEvent::Fault { .. }
                | CustodyEvent::Uncertain { .. }
                | CustodyEvent::Stopped { .. } => {
                    self.queue_pending(event)?;
                    return Err(io::Error::other("scope launch failed"));
                }
                _ => self.queue_pending(event)?,
            }
        }
    }

    fn queue_pending(&mut self, event: CustodyEvent) -> io::Result<()> {
        if self.pending.len() >= EVENT_CAPACITY {
            self.status.lock().unwrap().failed = true;
            self.batch_stop.store(true, Ordering::Release);
            return Err(io::Error::other("scope custody event queue overflow"));
        }
        self.pending.push_back(event);
        Ok(())
    }

    fn receive(&mut self, timeout: Duration) -> io::Result<Option<CustodyEvent>> {
        let event = match self.events.recv_timeout(timeout) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("custody broker lost"));
            }
        };
        Ok(Some(match event {
            CustodyEvent::ScopeEvent { scope, event } if scope == self.scope => match event {
                GroupEvent::Started { id, pid, group } => CustodyEvent::Started { id, pid, group },
                GroupEvent::Exited { id, status } => CustodyEvent::Exited { id, status },
                GroupEvent::Clean { id, forced } => CustodyEvent::Clean { id, forced },
                GroupEvent::Fault { reason } | GroupEvent::Rejected { reason, .. } => {
                    CustodyEvent::Fault { reason }
                }
                GroupEvent::Uncertain { id, reason } => CustodyEvent::Uncertain { id, reason },
            },
            CustodyEvent::ScopeStopped { scope, clean } if scope == self.scope => {
                CustodyEvent::Stopped { clean }
            }
            CustodyEvent::Fault { .. } | CustodyEvent::Uncertain { .. } => event,
            _ => return Err(io::Error::other("unexpected scoped custody event")),
        }))
    }

    fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<CustodyEvent>> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        self.receive(timeout)
    }
}

impl Drop for ScopedCustody {
    fn drop(&mut self) {
        self.signals.lost.store(true, Ordering::Release);
    }
}

struct Entry {
    events: SyncSender<CustodyEvent>,
    signals: Arc<Signals>,
    phase: ScopePhase,
    groups: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopePhase {
    Registering,
    Running,
    Stopping,
    Stopped,
    Rejected,
}

impl ScopePhase {
    fn terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Rejected)
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "custody deadline expired"))
}

fn fail(status: &Mutex<BrokerStatus>, stop: &AtomicBool) {
    status.lock().unwrap().failed = true;
    stop.store(true, Ordering::Release);
}

fn deliver(entry: &Entry, event: CustodyEvent, status: &Mutex<BrokerStatus>, stop: &AtomicBool) {
    if entry.signals.lost.load(Ordering::Acquire) {
        return;
    }
    match entry.events.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Disconnected(_)) => entry.signals.lost.store(true, Ordering::Release),
        Err(TrySendError::Full(_)) => fail(status, stop),
    }
}

fn serve(
    mut custody: Custody,
    requests: Receiver<Request>,
    stop: &AtomicBool,
    status: &Mutex<BrokerStatus>,
) {
    let mut scopes = BTreeMap::<String, Entry>::new();
    let mut stop_sent = false;
    let result = (|| -> io::Result<()> {
        loop {
            if stop.load(Ordering::Acquire) && !stop_sent {
                custody.stop()?;
                stop_sent = true;
            }
            for (scope, entry) in &mut scopes {
                if !entry.phase.terminal() && entry.signals.lost.load(Ordering::Acquire) {
                    status.lock().unwrap().failed = true;
                }
                if entry.phase == ScopePhase::Running
                    && (entry.signals.stop.load(Ordering::Acquire)
                        || entry.signals.lost.load(Ordering::Acquire))
                {
                    if entry.signals.lost.load(Ordering::Acquire) {
                        status.lock().unwrap().failed = true;
                    }
                    custody.request(
                        &OwnerRequest::StopScope {
                            scope: scope.clone(),
                        },
                        &[],
                    )?;
                    entry.phase = ScopePhase::Stopping;
                }
            }
            match requests.try_recv() {
                Ok(Request::Register {
                    scope,
                    deadline,
                    cleanup,
                    events,
                    signals,
                }) => {
                    if stop_sent || status.lock().unwrap().failed {
                        let _ = events.try_send(CustodyEvent::ScopeRejected {
                            scope,
                            reason: "scope admission closed".to_owned(),
                        });
                    } else if let std::collections::btree_map::Entry::Vacant(slot) =
                        scopes.entry(scope.clone())
                    {
                        let Ok(remaining) = remaining(deadline) else {
                            status.lock().unwrap().failed = true;
                            let _ = events.try_send(CustodyEvent::ScopeRejected {
                                scope,
                                reason: "scope deadline expired".to_owned(),
                            });
                            continue;
                        };
                        let milliseconds = remaining.as_millis().max(1);
                        let session_timeout_ms = u64::try_from(milliseconds)
                            .map_err(|_| io::Error::other("scope deadline overflow"))?;
                        let cleanup_timeout_ms = u64::try_from(cleanup.as_millis())
                            .map_err(|_| io::Error::other("scope cleanup overflow"))?;
                        custody.request(
                            &OwnerRequest::RegisterScope {
                                scope: scope.clone(),
                                session_timeout_ms,
                                cleanup_timeout_ms,
                            },
                            &[],
                        )?;
                        slot.insert(Entry {
                            events,
                            signals,
                            phase: ScopePhase::Registering,
                            groups: BTreeSet::new(),
                        });
                    } else {
                        fail(status, stop);
                    }
                }
                Ok(Request::Spawn { key, spec, stdio }) => {
                    if let Some(entry) = key.scope.as_ref().and_then(|scope| scopes.get_mut(scope))
                    {
                        if stop_sent {
                            deliver(
                                entry,
                                CustodyEvent::Fault {
                                    reason: "batch custody stopped".to_owned(),
                                },
                                status,
                                stop,
                            );
                        } else if matches!(
                            entry.phase,
                            ScopePhase::Registering | ScopePhase::Rejected
                        ) || (!entry.groups.insert(key.id.clone())
                            && entry.phase == ScopePhase::Running)
                        {
                            fail(status, stop);
                        } else {
                            custody.request(
                                &OwnerRequest::Spawn {
                                    scope: key.scope,
                                    id: key.id,
                                    spec,
                                },
                                &[stdio[0].as_fd(), stdio[1].as_fd(), stdio[2].as_fd()],
                            )?;
                        }
                    } else {
                        fail(status, stop);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    stop.store(true, Ordering::Release);
                }
                Err(TryRecvError::Empty) => {}
            }
            let Some(event) = custody.poll_event(TICK)? else {
                continue;
            };
            if dispatch_event(event, &mut scopes, status, stop) {
                return Ok(());
            }
        }
    })();
    let mut state = status.lock().unwrap();
    if result.is_err() {
        state.failed = true;
        state.uncertain = true;
    }
    state.finished = true;
}

fn dispatch_event(
    event: CustodyEvent,
    scopes: &mut BTreeMap<String, Entry>,
    status: &Mutex<BrokerStatus>,
    stop: &AtomicBool,
) -> bool {
    match &event {
        CustodyEvent::ScopeRegistered { scope } => {
            if let Some(entry) = scopes.get_mut(scope) {
                if entry.phase != ScopePhase::Registering {
                    fail(status, stop);
                } else {
                    entry.phase = ScopePhase::Running;
                    deliver(entry, event, status, stop);
                }
            } else {
                fail(status, stop);
            }
        }
        CustodyEvent::ScopeRejected { scope, .. } => {
            if let Some(entry) = scopes.get_mut(scope) {
                if entry.phase != ScopePhase::Registering {
                    fail(status, stop);
                } else {
                    entry.phase = ScopePhase::Rejected;
                    status.lock().unwrap().failed = true;
                    deliver(entry, event, status, stop);
                }
            } else {
                fail(status, stop);
            }
        }
        CustodyEvent::ScopeEvent {
            scope,
            event: group,
        } => {
            if let Some(entry) = scopes.get_mut(scope) {
                let id = match group {
                    GroupEvent::Started { id, .. }
                    | GroupEvent::Exited { id, .. }
                    | GroupEvent::Clean { id, .. }
                    | GroupEvent::Rejected { id, .. } => Some(id),
                    GroupEvent::Uncertain { id, .. } => id.as_ref(),
                    GroupEvent::Fault { .. } => None,
                };
                if id.is_some_and(|id| !entry.groups.contains(id))
                    || matches!(entry.phase, ScopePhase::Registering | ScopePhase::Rejected)
                    || (entry.phase == ScopePhase::Stopped
                        && !matches!(group, GroupEvent::Rejected { .. }))
                {
                    fail(status, stop);
                    return false;
                }
                if matches!(group, GroupEvent::Fault { .. }) {
                    entry.phase = ScopePhase::Stopping;
                    status.lock().unwrap().failed = true;
                }
                if matches!(group, GroupEvent::Uncertain { .. }) {
                    status.lock().unwrap().uncertain = true;
                    fail(status, stop);
                }
                deliver(entry, event, status, stop);
            } else {
                fail(status, stop);
            }
        }
        CustodyEvent::ScopeStopped { scope, clean } => {
            if let Some(entry) = scopes.get_mut(scope) {
                if !matches!(entry.phase, ScopePhase::Running | ScopePhase::Stopping) || !clean {
                    fail(status, stop);
                } else {
                    entry.phase = ScopePhase::Stopped;
                    deliver(entry, event, status, stop);
                }
            } else {
                fail(status, stop);
            }
        }
        CustodyEvent::Stopped { clean } => {
            let mut state = status.lock().unwrap();
            state.stopped = *clean;
            state.uncertain = !clean;
            state.failed |= !clean || scopes.values().any(|entry| !entry.phase.terminal());
            return true;
        }
        CustodyEvent::Fault { .. } | CustodyEvent::Uncertain { .. } => {
            if matches!(event, CustodyEvent::Uncertain { .. }) {
                status.lock().unwrap().uncertain = true;
            }
            fail(status, stop);
            for entry in scopes.values() {
                deliver(entry, event.clone(), status, stop);
            }
        }
        _ => fail(status, stop),
    }
    false
}
