use super::*;
use crate::session_owner_loss::{Process, observe};
use std::os::fd::{AsRawFd, FromRawFd};

struct Actor {
    process: Process,
    descriptor: OwnedFd,
}

impl Actor {
    fn pin(pid: i32, parent: i32) -> Self {
        let before = observe(pid).unwrap().expect("fixture actor must be live");
        assert_eq!(before.parent, parent);
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(raw >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        let descriptor = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        assert!(before.alive(), "fixture actor changed during pinning");
        assert_eq!(observe(pid).unwrap().unwrap().parent, parent);
        let mut status = libc::pollfd {
            fd: descriptor.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut status, 1, 0) }, 0);
        Self {
            process: before,
            descriptor,
        }
    }

    fn signal(&self, signal: i32) -> std::io::Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.descriptor.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn terminated(&self) -> bool {
        let mut descriptor = libc::pollfd {
            fd: self.descriptor.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert!(unsafe { libc::poll(&mut descriptor, 1, 0) } >= 0);
        descriptor.revents & libc::POLLIN != 0
    }
}

impl Drop for Actor {
    fn drop(&mut self) {
        let _ = self.signal(libc::SIGKILL);
    }
}

fn until<T>(description: &str, mut observe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(value) = observe() {
            return value;
        }
        assert!(Instant::now() < deadline, "{description}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn uncertain_publication_returns_a_failed_report_without_waiting_for_clean_custody() {
    let fixture = Fixture::new();
    fixture.1.set(true);
    short_budget(&fixture);
    fixture.write("payload.sh", b"printf completed\n");
    fixture.document("large.md", "large", "expected");
    let temporary = fixture.0.join("temporary");
    fs::create_dir(&temporary).unwrap();
    let (_unread, mut writer) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    loop {
        match writer.write(&[0; 65536]) {
            Ok(size) => assert!(size > 0),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => panic!("cannot fill fixture stdout: {error}"),
        }
    }
    writer.set_nonblocking(false).unwrap();
    let mut batch = OwnedBatch(
        fixture
            .command("1", &["large.md"])
            .env("TMPDIR", &temporary)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(fs::File::create(fixture.0.join("stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let (status_path, state, anchor_pid) = until("publication anchor was not admitted", || {
        for directory in fs::read_dir(&temporary).unwrap() {
            let path = directory.unwrap().path().join("custody.status.json");
            let Some(state) = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            else {
                continue;
            };
            let anchor =
                state["groups"].as_array().unwrap().iter().find(|group| {
                    group["scope"] == "publication" && group["id"] == "publication-0"
                });
            if let Some(anchor) = anchor {
                let pid = anchor["anchor_pid"].as_i64().unwrap() as i32;
                return Some((path, state, pid));
            }
        }
        None
    });
    let custodian = Actor::pin(
        state["custodian_pid"].as_i64().unwrap() as i32,
        batch.0.id() as i32,
    );
    let anchor = Actor::pin(anchor_pid, custodian.process.pid);
    let payload_pid = until("publication payload was not started", || {
        fs::read_to_string(format!("/proc/{anchor_pid}/task/{anchor_pid}/children"))
            .ok()?
            .split_whitespace()
            .next()?
            .parse::<i32>()
            .ok()
    });
    let payload = Actor::pin(payload_pid, anchor_pid);
    until("publication did not block writing stdout", || {
        let call = fs::read_to_string(format!("/proc/{payload_pid}/syscall")).ok()?;
        let mut fields = call.split_whitespace();
        (fields.next()?.parse::<i64>().ok()? == libc::SYS_write && fields.next()? == "0x1")
            .then_some(())
    });
    anchor.signal(libc::SIGSTOP).unwrap();
    let retained = until("publication custody did not become uncertain", || {
        let state: Value = serde_json::from_slice(&fs::read(&status_path).ok()?).ok()?;
        let publication_uncertain = state["groups"].as_array()?.iter().any(|group| {
            group["scope"] == "publication"
                && group["id"] == "publication-0"
                && group["uncertain"] == true
                && group["clean"] == false
        });
        (state["status"] == "custody_retained" && publication_uncertain).then_some(state)
    });
    fixture.write(
        "retained-before-report.json",
        serde_json::to_vec(&retained).unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    let status = loop {
        if let Some(status) = batch.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "uncertain publication blocked batch completion; evidence: {}; custody: {retained}",
            fixture.0.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!status.success());
    let report = fixture.report();
    assert_eq!(report["cleanup"]["outcome"], "uncertain", "{report}");
    assert_eq!(report["outcome"], "failure");
    assert!(report["infrastructure_failure"].is_string());
    assert_eq!(
        report["documents"][0]["session"]["cleanup"]["outcome"],
        "clean"
    );
    assert!(
        !custodian.terminated(),
        "uncertain custody must remain held"
    );
    assert_eq!(report["cleanup"]["status_path"], json!(status_path));
    for actor in [&payload, &anchor, &custodian] {
        if !actor.terminated() {
            actor.signal(libc::SIGKILL).unwrap();
        }
        until("fixture-owned actor survived teardown", || {
            actor.terminated().then_some(())
        });
    }
    fixture.1.set(false);
}
