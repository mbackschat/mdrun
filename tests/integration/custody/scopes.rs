use super::*;

fn register(controller: &mut Controller, scope: &str, timeout: u64) {
    controller.send(json!({"kind":"register_scope","scope":scope,"session_timeout_ms":timeout,"cleanup_timeout_ms":2000}), &[]);
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_registered","scope":scope})
    );
}

fn launch(controller: &mut Controller, scope: &str, id: &str, command: &str) -> Value {
    let output = File::create(controller.directory.join(format!("{scope}-{id}.output"))).unwrap();
    let input = File::open("/dev/null").unwrap();
    controller.send(
        json!({"kind":"spawn","scope":scope,"id":id,"spec":spec(command)}),
        &[input.as_raw_fd(), output.as_raw_fd(), output.as_raw_fd()],
    );
    let event = controller.receive();
    assert_eq!(event["kind"], "scope_event", "{event}");
    assert_eq!(event["scope"], scope, "{event}");
    assert_eq!(event["event"]["kind"], "started", "{event}");
    assert_eq!(event["event"]["id"], id, "{event}");
    if let Some(process) = observe(event["event"]["pid"].as_i64().unwrap() as i32).unwrap() {
        controller.payloads.push(process);
    }
    event["event"].clone()
}

fn drain_scope(controller: &mut Controller, scope: &str) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let event = controller.receive();
        let stopped = event["kind"] == "scope_stopped";
        events.push(event);
        // libtest retains this history even if the next receive or fixture cleanup panics.
        eprintln!("draining scope {scope}: {}", events.last().unwrap());
        assert_eq!(
            events.last().unwrap()["scope"],
            scope,
            "unexpected sibling event; history: {events:#?}; custody snapshot: {:?}; payload snapshots: {:?}",
            fs::read_to_string(controller.directory.join("custody.status.json")),
            controller
                .payloads
                .iter()
                .map(|process| (process, observe(process.pid)))
                .collect::<Vec<_>>()
        );
        if stopped {
            return events;
        }
    }
}

fn stop_scope(controller: &mut Controller, scope: &str) -> Vec<Value> {
    controller.send(json!({"kind":"stop_scope","scope":scope}), &[]);
    drain_scope(controller, scope)
}

fn complete_group(controller: &mut Controller, scope: &str, id: &str) {
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_event","scope":scope,"event":{"kind":"exited","id":id,"status":{"code":0,"signal":null}}})
    );
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_event","scope":scope,"event":{"kind":"clean","id":id,"forced":false}})
    );
}

fn finish(controller: &mut Controller, success: bool) {
    controller.send(json!({"kind":"stop"}), &[]);
    assert_eq!(controller.receive(), json!({"kind":"stopped","clean":true}));
    assert_eq!(controller.child.wait().unwrap().success(), success);
}

#[test]
fn scoped_groups_reuse_names_and_keep_one_lease_across_completed_scopes() {
    let directory = Directory::new();
    let reservation = super::super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    register(&mut controller, "first", 20000);
    register(&mut controller, "second", 20000);
    let first = launch(
        &mut controller,
        "first",
        "author",
        "printf first; exec /bin/sleep 15",
    );
    let second = launch(
        &mut controller,
        "second",
        "author",
        "printf second; exec /bin/sleep 15",
    );
    assert_ne!(first["group"], second["group"]);
    let first_payload = Process::record(first["pid"].as_i64().unwrap() as i32);
    let sibling = Process::record(second["pid"].as_i64().unwrap() as i32);
    assert_eq!(first_payload.group, first["group"].as_i64().unwrap() as i32);
    assert_eq!(sibling.group, second["group"].as_i64().unwrap() as i32);
    assert_eq!(first_payload.parent, first_payload.group);
    assert_eq!(sibling.parent, sibling.group);
    eprintln!("scope payloads before stop: {first_payload:?}; {sibling:?}");
    wait_until(|| {
        fs::read(directory.0.join("first-author.output")).is_ok_and(|bytes| bytes == b"first")
            && fs::read(directory.0.join("second-author.output"))
                .is_ok_and(|bytes| bytes == b"second")
    });
    let effect = directory.0.join("forbidden");
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(json!({"kind":"stop_scope","scope":"first"}), &[]);
    controller.send(json!({"kind":"spawn","scope":"first","id":"late","spec":spec(&format!("touch '{}'", effect.display()))}), &[descriptor.as_raw_fd();3]);
    controller.send(
        json!({"kind":"stop_group","scope":"first","id":"author"}),
        &[],
    );
    let events = drain_scope(&mut controller, "first");
    for id in ["late", "author"] {
        assert!(events.contains(&json!({"kind":"scope_event","scope":"first","event":{"kind":"rejected","id":id,"reason":"scope admission closed"}})));
    }
    assert!(!effect.exists());
    assert!(
        events
            .iter()
            .any(|event| event["event"]["kind"] == "exited")
    );
    assert!(events.iter().any(|event| event["event"]["kind"] == "clean"));
    assert_eq!(
        events.last().unwrap(),
        &json!({"kind":"scope_stopped","scope":"first","clean":true})
    );
    assert!(sibling.alive().unwrap());
    assert!(excluded(address));
    stop_scope(&mut controller, "second");
    assert!(excluded(address));
    assert_eq!(
        fs::read(directory.0.join("first-author.output")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(directory.0.join("second-author.output")).unwrap(),
        b"second"
    );
    register(&mut controller, "later", 20000);
    launch(&mut controller, "later", "author", "printf later");
    complete_group(&mut controller, "later", "author");
    stop_scope(&mut controller, "later");
    assert_eq!(
        fs::read(directory.0.join("later-author.output")).unwrap(),
        b"later"
    );
    assert!(excluded(address));
    finish(&mut controller, true);
    assert!(!excluded(address));
}

#[test]
fn expired_scope_rejects_late_requests_without_aborting_a_running_sibling() {
    let directory = Directory::new();
    let reservation = super::super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    register(&mut controller, "healthy", 20000);
    let healthy = launch(&mut controller, "healthy", "author", "exec /bin/sleep 15");
    register(&mut controller, "short", 200);
    let events = drain_scope(&mut controller, "short");
    assert!(
        events.iter().any(
            |event| event["event"] == json!({"kind":"fault","reason":"scope deadline expired"})
        )
    );
    let effect = directory.0.join("forbidden");
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(json!({"kind":"spawn","scope":"short","id":"late","spec":spec(&format!("touch '{}'", effect.display()))}), &[descriptor.as_raw_fd();3]);
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_event","scope":"short","event":{"kind":"rejected","id":"late","reason":"scope admission closed"}})
    );
    controller.send(
        json!({"kind":"stop_group","scope":"short","id":"late"}),
        &[],
    );
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_event","scope":"short","event":{"kind":"rejected","id":"late","reason":"scope admission closed"}})
    );
    assert!(!effect.exists());
    controller.send(json!({"kind":"register_scope","scope":"not-started","session_timeout_ms":20000,"cleanup_timeout_ms":2000}), &[]);
    assert_eq!(
        controller.receive(),
        json!({"kind":"scope_rejected","scope":"not-started","reason":"scope admission closed"})
    );
    assert!(
        Process::record(healthy["pid"].as_i64().unwrap() as i32)
            .alive()
            .unwrap()
    );
    launch(&mut controller, "healthy", "next", "printf healthy-next");
    complete_group(&mut controller, "healthy", "next");
    stop_scope(&mut controller, "healthy");
    assert_eq!(
        fs::read(directory.0.join("healthy-next.output")).unwrap(),
        b"healthy-next"
    );
    assert!(excluded(address));
    finish(&mut controller, false);
    assert!(!excluded(address));
}

#[test]
fn owner_control_loss_sweeps_all_scopes_before_releasing_the_shared_listener() {
    let directory = Directory::new();
    let reservation = super::super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    register(&mut controller, "first", 20000);
    register(&mut controller, "second", 20000);
    let first = launch(&mut controller, "first", "author", "exec /bin/sleep 15");
    let second = launch(&mut controller, "second", "author", "exec /bin/sleep 15");
    let payloads =
        [first, second].map(|event| Process::record(event["pid"].as_i64().unwrap() as i32));
    assert!(payloads.iter().all(|process| process.alive().unwrap()));
    assert!(excluded(address));
    controller
        .stream
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    wait_until(|| !excluded(address));
    assert!(payloads.iter().all(|process| !process.alive().unwrap()));
    wait_until(|| {
        fs::read(directory.0.join("custody.status.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|state| state["status"] == "released")
    });
    let state: Value =
        serde_json::from_slice(&fs::read(directory.0.join("custody.status.json")).unwrap())
            .unwrap();
    assert_eq!(state["status"], "released");
    assert!(
        state["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["clean"] == true)
    );
    assert_eq!(state["scopes"].as_array().unwrap().len(), 2);
    controller.released = true;
}

#[test]
fn scoped_launch_failure_is_sticky_but_a_healthy_sibling_can_finish() {
    let directory = Directory::new();
    let reservation = super::super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    register(&mut controller, "failed", 20000);
    register(&mut controller, "healthy", 20000);
    let healthy = launch(&mut controller, "healthy", "author", "exec /bin/sleep 15");
    let mut missing = spec("");
    missing["argv"] = json!([os(format!("/mdrun-missing-{}", uuid::Uuid::new_v4()))]);
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(
        json!({"kind":"spawn","scope":"failed","id":"author","spec":missing}),
        &[descriptor.as_raw_fd(); 3],
    );
    let events = drain_scope(&mut controller, "failed");
    assert!(events.contains(&json!({"kind":"scope_event","scope":"failed","event":{"kind":"fault","reason":"payload could not be started"}})));
    assert!(events.contains(&json!({"kind":"scope_event","scope":"failed","event":{"kind":"clean","id":"author","forced":false}})));
    assert!(
        Process::record(healthy["pid"].as_i64().unwrap() as i32)
            .alive()
            .unwrap()
    );
    assert!(excluded(address));
    launch(&mut controller, "healthy", "next", "printf healthy");
    complete_group(&mut controller, "healthy", "next");
    stop_scope(&mut controller, "healthy");
    assert_eq!(
        fs::read(directory.0.join("healthy-next.output")).unwrap(),
        b"healthy"
    );
    finish(&mut controller, false);
    assert!(!excluded(address));
}

#[test]
fn malformed_owner_packet_fails_while_clean_eof_releases_empty_custody() {
    for malformed in [true, false] {
        let directory = Directory::new();
        let reservation = super::super::ports::Reservation::new();
        let address = reservation.address();
        let mut controller = Controller::start(&directory.0, reservation, 30000);
        if malformed {
            controller.send(json!({"kind":"spawn","id":"never-launched"}), &[]);
            assert_eq!(
                controller.receive(),
                json!({"kind":"fault","reason":"invalid custody control packet"})
            );
        } else {
            controller
                .stream
                .shutdown(std::net::Shutdown::Write)
                .unwrap();
        }
        wait_until(|| !excluded(address));
        let success = controller.child.wait().unwrap().success();
        let state: Value =
            serde_json::from_slice(&fs::read(directory.0.join("custody.status.json")).unwrap())
                .unwrap();
        assert_eq!(state["status"], "released");
        assert_eq!(state["groups"], json!([]));
        controller.released = true;
        assert_eq!(success, !malformed);
    }
}

#[test]
fn invalid_scope_control_stops_the_batch_without_starting_a_payload() {
    for case in [
        "unknown",
        "duplicate",
        "mixed",
        "malformed",
        "duplicate_after_stop",
    ] {
        let directory = Directory::new();
        let reservation = super::super::ports::Reservation::new();
        let address = reservation.address();
        let mut controller = Controller::start(&directory.0, reservation, 30000);
        register(&mut controller, "document", 20000);
        if case == "duplicate_after_stop" {
            stop_scope(&mut controller, "document");
        }
        let descriptor = File::open("/dev/null").unwrap();
        let effect = directory.0.join("forbidden");
        let launch = spec(&format!("touch '{}'", effect.display()));
        match case {
            "unknown" => controller.send(json!({"kind":"spawn","scope":"unknown","id":"author","spec":launch}), &[descriptor.as_raw_fd();3]),
            "mixed" => controller.send(json!({"kind":"spawn","id":"author","spec":launch}), &[descriptor.as_raw_fd();3]),
            "malformed" => controller.send(json!({"kind":"register_scope","scope":"other","session_timeout_ms":true,"cleanup_timeout_ms":2000}), &[]),
            _ => controller.send(json!({"kind":"register_scope","scope":"document","session_timeout_ms":20000,"cleanup_timeout_ms":2000}), &[]),
        }
        let fault = controller.receive();
        assert!(
            matches!(fault["kind"].as_str(), Some("fault" | "uncertain")),
            "{case}: {fault}"
        );
        if case == "malformed" {
            wait_until(|| !excluded(address));
            assert!(!controller.child.wait().unwrap().success());
            controller.released = true;
        } else {
            loop {
                let event = controller.receive();
                if event["kind"] == "stopped" {
                    assert_eq!(event["clean"], true);
                    break;
                }
                assert_eq!(
                    event,
                    json!({"kind":"scope_stopped","scope":"document","clean":true})
                );
            }
            assert!(!controller.child.wait().unwrap().success());
        }
        assert!(!effect.exists(), "{case} performed a payload effect");
        assert!(!excluded(address));
    }
}

#[test]
fn scoped_anchor_uncertainty_keeps_the_batch_excluded_without_clean_completion() {
    use std::os::unix::ffi::OsStrExt;
    let directory = Directory::new();
    let reservation = super::super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    register(&mut controller, "failed", 20000);
    register(&mut controller, "sibling", 20000);
    let executable = std::env::current_exe().unwrap();
    let mut launch = spec("");
    launch["argv"] = json!([
        os(executable.as_os_str().as_bytes()),
        os("--exact"),
        os("custody::cooperative_payload_fixture"),
        os("--nocapture"),
        os("--test-threads=1")
    ]);
    launch["environment"] = json!([[
        os("MDRUN_CUSTODY_PAYLOAD_DIRECTORY"),
        os(directory.0.as_os_str().as_bytes())
    ]]);
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(
        json!({"kind":"spawn","scope":"failed","id":"author","spec":launch}),
        &[descriptor.as_raw_fd(); 3],
    );
    let started = controller.receive();
    assert_eq!(started["scope"], "failed");
    assert_eq!(started["event"]["kind"], "started");
    wait_until(|| directory.0.join("ready").exists());
    let payload = Process::record(started["event"]["pid"].as_i64().unwrap() as i32);
    controller.payloads.push(payload.clone());
    let anchor = Process::record(started["event"]["group"].as_i64().unwrap() as i32);
    controller.kill_anchor(&anchor).unwrap();
    let event = controller.receive();
    assert_eq!(event["scope"], "failed");
    assert_eq!(event["event"]["kind"], "uncertain");
    assert!(payload.alive().unwrap());
    assert!(excluded(address));
    wait_until(|| {
        fs::read(directory.0.join("custody.status.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|state| {
                state["status"] == "custody_retained" && state["scopes"][0]["phase"] == "stopping"
            })
    });
    let state: Value =
        serde_json::from_slice(&fs::read(directory.0.join("custody.status.json")).unwrap())
            .unwrap();
    assert_eq!(state["groups"][0]["scope"], "failed");
    assert_eq!(state["groups"][0]["clean"], false);
    drop(controller);
    assert!(!payload.alive().unwrap());
    assert!(!excluded(address));
    assert!(!directory.0.join("expired").exists());
}
