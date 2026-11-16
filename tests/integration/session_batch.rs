use super::{Fixture, node};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

struct ReleaseGates<'a>(&'a Path);

impl Drop for ReleaseGates<'_> {
    fn drop(&mut self) {
        for id in ["a", "b", "c"] {
            for gate in ["invoke-go", "close-go"] {
                let _ = fs::write(self.0.join(format!("{id}.{gate}")), b"");
            }
        }
    }
}

fn read_state(directory: &Path, name: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(bytes) = fs::read(directory.join(name)) {
            return serde_json::from_slice(&bytes).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "missing {name}: {}",
            String::from_utf8_lossy(&fs::read(directory.join("stderr")).unwrap_or_default())
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn listener_state(worker: &Value) -> Value {
    let address = SocketAddr::from(([127, 0, 0, 1], worker["port"].as_u64().unwrap() as u16));
    let stream = TcpStream::connect_timeout(&address, Duration::from_millis(500)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let state: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(state["session_id"], worker["session_id"]);
    state
}

fn assert_excluded(address: SocketAddr) {
    assert_eq!(
        TcpListener::bind(address).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse
    );
}

fn worker_count(directory: &Path) -> usize {
    fs::read_dir(directory)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".worker.json")
        })
        .count()
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn warm_slots_cover_worker_shutdown_and_cleanup_before_queued_admission() {
    let fixture = Fixture::new();
    let runtime = node();
    assert!(runtime.is_absolute() && runtime.is_file());
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut reservation = crate::ports::Reservation::new();
    let address = reservation.address();
    fixture.write(
        "binding.json",
        serde_json::to_vec(&json!({
            "version":1,"name":"tool","artifact_id":"node-batch-slot-fixture-v1",
            "worker_argv":[runtime,source.join("tests/runtime/node/batch-worker.mjs")],
            "cold_argv":["/mdrun-cold-must-not-start"],"warm_ignore_env":["SHLVL","_"],
            "context":{"environment":{"PWD":fixture.0.to_str().unwrap()}},
            "startup_timeout_ms":5000,"session_timeout_ms":20000,"cleanup_timeout_ms":5000,
            "resources":[{"kind":"tcp_listener","host":"127.0.0.1","port":address.port()}]
        }))
        .unwrap(),
    );
    let originals: Vec<_> = ["a", "b", "c"]
        .into_iter()
        .map(|id| {
            let name = format!("{id}.md");
            let bytes = format!(
                "```sh\ntool {id} first\n```\n\n```output\n{id}:first\n```\n\n```sh\ntool {id} second\n```\n\n```output\n{id}:second\n```\n"
            );
            fixture.write(&name, &bytes);
            (name, bytes)
        })
        .collect();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command.current_dir(&fixture.0).args([
        "verify-many",
        "--binding",
        "binding.json",
        "--jobs",
        "2",
        "--require-warm",
        "--share-resources",
        "--report",
        "report.json",
        "--",
        "a.md",
        "b.md",
        "c.md",
    ]);
    reservation.release_listener();
    let output = std::thread::scope(|scope| {
        let capture = scope.spawn(|| fixture.capture(command));
        {
            let _release = ReleaseGates(&fixture.0);
            let first = read_state(&fixture.0, "a.second.json");
            drop(reservation);
            let healthy = read_state(&fixture.0, "b.second.json");
            assert_ne!(first["pid"], healthy["pid"]);
            assert_ne!(first["session_id"], healthy["session_id"]);
            assert_excluded(address);
            assert!(!fixture.0.join("c.first.json").exists());
            fixture.write("a.invoke-go", b"");
            let shutdown = read_state(&fixture.0, "a.shutdown.json");
            assert_eq!(shutdown["calls"], json!(["first", "second"]));
            assert_eq!(shutdown["session_id"], first["session_id"]);
            let first_tick = listener_state(&first)["ticks"].as_u64().unwrap();
            let healthy_tick = listener_state(&healthy)["ticks"].as_u64().unwrap();
            // Orderly SDK shutdown follows author completion, but bootstrap still owns this listener.
            let observation = Instant::now() + Duration::from_millis(250);
            while Instant::now() < observation {
                assert_eq!(
                    worker_count(&fixture.0),
                    2,
                    "queued worker started during cleanup"
                );
                assert!(!fixture.0.join("c.first.json").exists());
                assert!(!fixture.0.join("a.closed.json").exists());
                assert_excluded(address);
                std::thread::sleep(Duration::from_millis(5));
            }
            let first_state = listener_state(&first);
            let healthy_state = listener_state(&healthy);
            assert_eq!(first_state["stopping"], true);
            assert_eq!(healthy_state["stopping"], false);
            assert!(first_state["ticks"].as_u64().unwrap() > first_tick);
            assert!(healthy_state["ticks"].as_u64().unwrap() > healthy_tick);
            fixture.write("a.close-go", b"");
            let queued = read_state(&fixture.0, "c.second.json");
            assert_eq!(worker_count(&fixture.0), 3);
            assert_ne!(queued["session_id"], first["session_id"]);
            assert_ne!(queued["session_id"], healthy["session_id"]);
            assert_eq!(listener_state(&healthy)["stopping"], false);
            assert!(!fixture.0.join("b.shutdown.json").exists());
            assert_excluded(address);
            fixture.write("b.invoke-go", b"");
            fixture.write("c.invoke-go", b"");
            read_state(&fixture.0, "b.shutdown.json");
            read_state(&fixture.0, "c.shutdown.json");
            fixture.write("b.close-go", b"");
            fixture.write("c.close-go", b"");
        }
        capture.join().unwrap()
    });
    assert!(
        output.status.success(),
        "warm batch failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let report = fixture.report();
    assert_eq!(report["outcome"], "success");
    assert_eq!(report["infrastructure_failure"], Value::Null);
    assert_eq!(report["cleanup"]["outcome"], "clean");
    assert_eq!(report["cleanup"]["status_path"], Value::Null);
    assert_eq!(
        report["concurrency"],
        json!({"requested":2,"effective":2,"share_resources":true})
    );
    let documents = report["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 3);
    let mut sessions = HashSet::new();
    for (index, (entry, id)) in documents.iter().zip(["a", "b", "c"]).enumerate() {
        assert_eq!(entry["index"], index);
        assert_eq!(entry["state"], "completed");
        let session = &entry["session"];
        assert_eq!(session["outcome"], "success");
        assert_eq!(session["author_status"]["exit_code"], 0);
        assert_eq!(session["warm_calls"], 2);
        assert_eq!(session["cold_calls"], 0);
        assert_eq!(session["cold_reasons"], json!({}));
        assert_eq!(session["worker_starts"], 1);
        assert_eq!(session["infrastructure_failure"], Value::Null);
        assert_eq!(session["cleanup"]["outcome"], "clean");
        assert_eq!(session["cleanup"]["status_path"], Value::Null);
        assert_eq!(
            session["cleanup"]["custodian_pid"],
            report["cleanup"]["custodian_pid"]
        );
        assert!(sessions.insert(session["session_id"].as_str().unwrap()));
        let statuses = session["application_statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 2);
        for status in statuses {
            assert_eq!(status["route"], "warm");
            assert_eq!(status["exit_code"], 0);
            assert_eq!(status["signal"], Value::Null);
        }
        let closed = read_state(&fixture.0, &format!("{id}.closed.json"));
        assert_eq!(closed["session_id"], session["session_id"]);
        assert!(!Path::new(closed["private_directory"].as_str().unwrap()).exists());
        let port = closed["port"].as_u64().unwrap() as u16;
        assert_eq!(
            TcpStream::connect_timeout(
                &SocketAddr::from(([127, 0, 0, 1], port)),
                Duration::from_millis(500)
            )
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::ConnectionRefused
        );
    }
    assert!(TcpListener::bind(address).is_ok());
    for entry in fs::read_dir(&fixture.0).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.ends_with(".expired") && !name.ends_with(".failed"),
            "fixture failed: {name}"
        );
    }
    for (name, original) in originals {
        assert_eq!(fs::read(fixture.0.join(name)).unwrap(), original.as_bytes());
    }
}
