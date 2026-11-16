use super::{Fixture, PARITY_CALLS, node, parity_mode_fixture, run_parity_author};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn packaged_cold_matches_required_warm_native_author() {
    let node = node();
    assert!(node.is_absolute() && node.is_file());
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let binding = json!({
        "version":1,"name":"tool","artifact_id":"node-sdk-fixture-v1",
        "worker_argv":[node,source.join("tests/runtime/node/fixture-worker.mjs")],
        "cold_argv":[node,source.join("tests/runtime/node/cold-app.mjs")],
        "startup_timeout_ms":5000,"session_timeout_ms":20000,"cleanup_timeout_ms":2000
    });
    let root = Fixture::new();
    let cold = parity_mode_fixture(&root, "cold", &binding);
    let fast = parity_mode_fixture(&root, "fast", &binding);
    let expected_statuses = [0, 17, 0, 0, 0, 0, 0];
    let echo_input = fs::read(cold.0.join("echo.input")).unwrap();
    let expected_stdout: [&[u8]; 7] = [
        b"ok",
        b"",
        &[0, 255, 1, 2],
        b"before application input changes\n",
        b"after application input changes\0\xff\n",
        &echo_input,
        b"ok",
    ];

    for (fixture, options, warm_calls, cold_calls, worker_starts) in [
        (&cold, &["--mode", "cold"][..], 0, 7, 0),
        (&fast, &["--mode", "fast", "--require-warm"][..], 7, 0, 1),
    ] {
        let output = run_parity_author(fixture, options);
        assert!(
            output.status.success(),
            "native author failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for (index, call) in PARITY_CALLS.iter().enumerate() {
            assert_eq!(
                fs::read(fixture.0.join(format!("{call}.stdout"))).unwrap(),
                expected_stdout[index],
                "stdout for {call}"
            );
            let expected_stderr: &[u8] = if index == 2 { &[254, 0] } else { b"" };
            assert_eq!(
                fs::read(fixture.0.join(format!("{call}.stderr"))).unwrap(),
                expected_stderr,
                "stderr for {call}"
            );
            assert_eq!(
                fs::read_to_string(fixture.0.join(format!("{call}.status"))).unwrap(),
                format!("{}\n", expected_statuses[index]),
                "status for {call}"
            );
        }
        let report = fixture.report();
        assert_eq!(report["artifact_id"], binding["artifact_id"]);
        assert_eq!(report["warm_calls"], warm_calls);
        assert_eq!(report["cold_calls"], cold_calls);
        assert_eq!(report["worker_starts"], worker_starts);
        if cold_calls == 0 {
            assert_eq!(report["cold_reasons"], json!({}));
        } else {
            assert_eq!(report["cold_reasons"]["explicit_cold"], cold_calls);
        }
        let statuses = report["application_statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), expected_statuses.len());
        for (actual, expected) in statuses.iter().zip(expected_statuses) {
            assert_eq!(actual["exit_code"], expected);
            assert_eq!(actual["signal"], Value::Null);
            assert_eq!(
                actual["route"],
                if cold_calls == 0 { "warm" } else { "cold" }
            );
        }
        assert_eq!(report["author_status"]["exit_code"], 0);
        assert_eq!(report["outcome"], "success");
        assert_eq!(report["infrastructure_failure"], Value::Null);
        assert_eq!(report["cleanup"]["outcome"], "clean");
    }

    for call in PARITY_CALLS {
        for suffix in ["stdout", "stderr", "status"] {
            let file = format!("{call}.{suffix}");
            assert_eq!(
                fs::read(cold.0.join(&file)).unwrap(),
                fs::read(fast.0.join(&file)).unwrap(),
                "packaged cold/warm parity for {file}"
            );
        }
    }
}
