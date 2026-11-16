use super::session_owner_loss::{Run, excluded, observe, read_json, wait_until, write_json};
use serde_json::json;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const AUTHOR: &str = "warm_owner_loss::author_fixture";
const MARKER: &str = "MDRUN_WARM_OWNER_LOSS_AUTHOR";

#[derive(Clone, Copy)]
enum Loss {
    Host,
    Worker,
    Author,
}

fn node_command() -> Command {
    let node = PathBuf::from(
        std::env::var_os("MDRUN_NODE")
            .expect("set MDRUN_NODE to the prepared absolute Node executable"),
    );
    assert!(node.is_absolute());
    let mut command = Command::new(node);
    command.env_clear().env("PATH", "/usr/bin:/bin");
    command.arg(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/owner-loss-worker.mjs"),
    );
    command
}

#[test]
#[ignore = "native author helper; no-op without its managed-session marker"]
fn author_fixture() {
    if std::env::var_os(MARKER).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    fs::write(
        "client-path.tmp",
        std::env::var_os("MDRUN_SESSION")
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();
    fs::rename("client-path.tmp", "client-path").unwrap();
    write_json(
        Path::new("author.ready.json"),
        &json!({"pid":std::process::id()}),
    );
    let mut client = Command::new("tool")
        .arg("hold")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create("client.stderr").unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Path::new("lose-author").exists() {
            assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGKILL) }, 0);
            unreachable!("SIGKILL returned to the native author");
        }
        if let Some(status) = client.try_wait().unwrap() {
            assert!(status.success(), "accepted warm client failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            fs::write("author.expired", b"").unwrap();
            let _ = client.kill();
            let _ = client.wait();
            panic!("native author exceeded its external deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires prepared Node runtime selected by MDRUN_NODE"]
fn host_sigkill_retains_declared_listener_until_warm_descendant_cleanup() {
    check_loss(Loss::Host);
}

#[test]
#[ignore = "requires prepared Node runtime selected by MDRUN_NODE"]
fn worker_sigkill_fails_session_and_retains_listener_until_descendant_cleanup() {
    check_loss(Loss::Worker);
}

#[test]
#[ignore = "requires prepared Node runtime selected by MDRUN_NODE"]
fn author_sigkill_fails_session_and_retains_listener_until_warm_descendant_cleanup() {
    check_loss(Loss::Author);
}

fn check_loss(loss: Loss) {
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let directory =
        std::env::temp_dir().join(format!("mdrun-warm-owner-control-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let mut command = node_command();
    command.env("MDRUN_OWNER_LOSS_ROLE", "direct_owner").env(
        "MDRUN_OWNER_LOSS_ADDRESS",
        json!({"host":"127.0.0.1","port":address.port()}).to_string(),
    );
    let mut control = Run::spawn(directory, &mut command);
    wait_until("direct Node owner readiness", || {
        control.directory.join("ready.json").exists()
    });
    let ready = read_json(&control.directory.join("ready.json"));
    let uncontrolled = observe(ready["descendant"].as_i64().unwrap() as i32)
        .unwrap()
        .unwrap();
    assert!(excluded(address));
    control.kill_owner();
    assert!(uncontrolled.alive());
    assert!(
        !excluded(address),
        "negative control unexpectedly retained the owner-only listener"
    );
    assert!(
        uncontrolled.alive(),
        "negative-control descendant exited during the probe"
    );
    assert!(!control.directory.join("owner.expired").exists());
    assert!(!control.directory.join("descendant.expired").exists());
    fs::write(control.directory.join("finished"), b"").unwrap();
    wait_until("negative-control descendant exit", || !uncontrolled.alive());
    drop(control);

    let directory =
        std::env::temp_dir().join(format!("mdrun-warm-owner-managed-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let worker = node_command();
    let worker_argv: Vec<_> = std::iter::once(worker.get_program())
        .chain(worker.get_args())
        .map(|argument| argument.to_str().unwrap().to_owned())
        .collect();
    write_json(
        &directory.join("binding.json"),
        &json!({
            "version":1,"name":"tool","artifact_id":"native-warm-owner-loss-fixture",
            "worker_argv":worker_argv,"cold_argv":["/mdrun-cold-must-not-start"],
            "resources":[{"kind":"tcp_listener","host":"127.0.0.1","port":address.port()}],
            "startup_timeout_ms":3000,"session_timeout_ms":15000,"cleanup_timeout_ms":2000
        }),
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(MARKER, "1")
        .args(["session", "--require-warm", "--binding"])
        .arg(directory.join("binding.json"))
        .arg("--report")
        .arg(directory.join("report.json"))
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            AUTHOR,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&directory);
    let mut managed = Run::spawn(directory, &mut command);
    wait_until("native author configuration", || {
        assert!(
            managed.child.try_wait().unwrap().is_none(),
            "session exited before author startup: {}",
            String::from_utf8_lossy(&fs::read(managed.directory.join("stderr")).unwrap())
        );
        managed.directory.join("client-path").exists()
    });
    let client = PathBuf::from(fs::read_to_string(managed.directory.join("client-path")).unwrap());
    assert!(client.is_absolute() && client.is_file());
    managed.private = Some(client.parent().unwrap().to_owned());
    let status_path = client.parent().unwrap().join("custody.status.json");
    wait_until("accepted warm invocation and descendant readiness", || {
        assert!(
            managed.child.try_wait().unwrap().is_none(),
            "session exited before warm invocation: {}",
            String::from_utf8_lossy(&fs::read(managed.directory.join("stderr")).unwrap())
        );
        managed.directory.join("ready.json").exists()
    });
    let ready = read_json(&managed.directory.join("ready.json"));
    let worker = observe(ready["worker"].as_i64().unwrap() as i32)
        .unwrap()
        .unwrap();
    let descendant = observe(ready["descendant"].as_i64().unwrap() as i32)
        .unwrap()
        .unwrap();
    let author = observe(
        read_json(&managed.directory.join("author.ready.json"))["pid"]
            .as_i64()
            .unwrap() as i32,
    )
    .unwrap()
    .unwrap();
    assert_eq!(descendant.parent, worker.pid);
    assert_eq!(descendant.group, worker.group);
    assert!(worker.alive() && descendant.alive() && author.alive());
    assert!(excluded(address));
    wait_until("worker and author custody registration", || {
        read_json(&status_path)["groups"]
            .as_array()
            .is_some_and(|groups| groups.len() == 2)
    });
    let status = read_json(&status_path);
    let groups = status["groups"].as_array().unwrap();
    assert!(
        groups
            .iter()
            .any(|group| group["id"] == "worker" && group["anchor_pid"] == worker.group)
    );
    assert!(
        groups
            .iter()
            .any(|group| group["id"] == "author" && group["anchor_pid"] == author.group)
    );
    assert!(!groups.iter().any(|group| {
        group["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("cold"))
    }));

    match loss {
        Loss::Host => managed.kill_owner(),
        Loss::Worker => {
            fs::write(managed.directory.join("lose-worker"), b"").unwrap();
            wait_until("worker self-SIGKILL", || !worker.alive());
        }
        Loss::Author => {
            fs::write(managed.directory.join("lose-author"), b"").unwrap();
            wait_until("native author self-SIGKILL", || !author.alive());
        }
    }
    assert!(
        descendant.alive(),
        "descendant did not survive actual host, worker or author death"
    );
    assert!(
        excluded(address),
        "host, worker or author death released the declared listener early"
    );
    let mut surviving_probes = 0;
    wait_until("warm descendant cleanup before listener release", || {
        let before = descendant.alive();
        let held = excluded(address);
        let after = descendant.alive();
        if before && after {
            surviving_probes += 1;
            assert!(
                held,
                "listener became available while the warm descendant survived"
            );
        }
        if !held {
            assert!(!after, "listener release preceded descendant cleanup");
            assert!(!worker.alive(), "worker survived listener release");
            assert!(!author.alive(), "author survived listener release");
        }
        !held && !before
    });
    assert!(
        surviving_probes > 0,
        "no post-owner-loss custody observation"
    );
    assert!(
        !managed.directory.join("owner.expired").exists(),
        "owner fixture self-expiry cannot establish custody cleanup"
    );
    assert!(
        !managed.directory.join("descendant.expired").exists(),
        "descendant self-expiry cannot establish custody cleanup"
    );
    assert!(
        !managed.directory.join("author.expired").exists(),
        "author self-expiry cannot establish custody cleanup"
    );
    match loss {
        Loss::Host => {
            wait_until("custodian released state", || {
                read_json(&status_path)["status"] == "released"
            });
            let status = read_json(&status_path);
            assert!(
                status["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|group| group["clean"] == true && group["uncertain"] == false),
                "{status}"
            );
            assert!(
                !managed.directory.join("report.json").exists(),
                "killed host unexpectedly completed reporting"
            );
        }
        Loss::Worker | Loss::Author => {
            let mut exit = None;
            wait_until("failed session exit after worker or author loss", || {
                exit = managed.child.try_wait().unwrap();
                exit.is_some()
            });
            assert!(!exit.unwrap().success());
            let report = read_json(&managed.directory.join("report.json"));
            assert_ne!(
                report["infrastructure_failure"],
                serde_json::Value::Null,
                "{report}"
            );
            match loss {
                Loss::Author => {
                    assert_eq!(report["author_status"]["signal"], libc::SIGKILL, "{report}")
                }
                Loss::Worker => assert!(
                    report["infrastructure_failure"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("worker")),
                    "{report}"
                ),
                Loss::Host => unreachable!(),
            }
            assert_eq!(report["cleanup"]["outcome"], "clean", "{report}");
            assert!(report["outcome"] != "success", "{report}");
            assert!(
                !client.parent().unwrap().exists(),
                "clean session retained its private directory"
            );
            managed.private = None;
        }
    }
}
