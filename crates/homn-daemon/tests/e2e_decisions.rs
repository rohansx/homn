//! End-to-end integration test for the daemon decisions pipeline (T022).
//!
//! Spins up `SocketServer` against a temp socket + temp audit DB + a real Rhai ruleset, then
//! verifies the three [User Story 1 acceptance scenarios] over a real Unix-socket connection.
//!
//! [User Story 1 acceptance scenarios]: ../../../specs/001-policy-engine/spec.md

use std::sync::Arc;
use std::time::Duration;

use homn_audit::Db;
use homn_daemon::{handler::DaemonState, socket::SocketServer};
use homn_policy::{Engine, RuleSet};
use homn_types::{Request, Response};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const POLICY: &str = r#"
deny  if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
allow if tool == "Read" && path.starts_with(home);
"#;

async fn spawn_daemon() -> (std::path::PathBuf, DaemonState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("homn.sock");

    let engine = Engine::new();
    let rules = RuleSet::parse(&engine, POLICY, "test.rhai").unwrap();
    let audit = Arc::new(Db::in_memory().await.unwrap());
    let state = DaemonState {
        engine,
        rules: Arc::new(rules),
        audit,
    };
    let state_for_serve = state.clone();

    let server = SocketServer::bind(&sock_path).await.unwrap();
    let bound = server.path().to_path_buf();
    tokio::spawn(async move {
        let _ = server.serve(state_for_serve).await;
    });

    // Give the listener a moment.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (bound, state, dir)
}

async fn send(sock: &std::path::Path, req: &Request) -> Response {
    let mut stream = UnixStream::connect(sock).await.unwrap();
    let line = format!("{}\n", serde_json::to_string(req).unwrap());
    stream.write_all(line.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    stream.shutdown().await.unwrap();

    let mut reader = BufReader::new(stream).lines();
    let resp_line = reader.next_line().await.unwrap().expect("got response");
    serde_json::from_str(&resp_line).unwrap()
}

fn decisions_create(id: &str, tool: &str, tool_input: serde_json::Value, cwd: &str) -> Request {
    Request {
        id: id.into(),
        method: "decisions.create".into(),
        params: json!({
            "source": "hook",
            "session_id": "01HXY",
            "cwd": cwd,
            "tool_name": tool,
            "tool_input": tool_input,
        }),
    }
}

#[tokio::test]
async fn us1_scenario_1_destructive_bash_is_denied_and_audited() {
    let (sock, state, _dir) = spawn_daemon().await;

    // Set HOME so the policy's "starts_with(home)" comparisons are reproducible.
    // Not strictly needed for this scenario but keeps tests deterministic.
    std::env::set_var("HOME", "/home/rsx");

    let req = decisions_create(
        "a",
        "Bash",
        json!({"command": "rm -rf ~/scratch"}),
        "/home/rsx/dev/cloakpipe",
    );
    let resp = send(&sock, &req).await;
    match resp {
        Response::Ok { id, result } => {
            assert_eq!(id, "a");
            assert_eq!(result["decision"], "deny");
            assert_eq!(result["rule_source"]["file"], "test.rhai");
            assert_eq!(result["rule_source"]["line"], 2);
        }
        Response::Err { error, .. } => panic!("expected Ok, got error: {error:?}"),
    }
    let rows = state.audit.tail(10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, homn_types::Decision::Deny);
    assert_eq!(rows[0].tool_name, "Bash");
}

#[tokio::test]
async fn us1_scenario_2_read_in_home_is_allowed_silently() {
    std::env::set_var("HOME", "/home/rsx");
    let (sock, state, _dir) = spawn_daemon().await;

    let req = decisions_create(
        "b",
        "Read",
        json!({"path": "/home/rsx/foo.txt"}),
        "/home/rsx/dev/x",
    );
    let resp = send(&sock, &req).await;
    match resp {
        Response::Ok { result, .. } => {
            assert_eq!(result["decision"], "allow");
            assert!(result["rule_source"].is_object());
            let latency = result["latency_ms"].as_u64().unwrap();
            assert!(latency < 50, "p95 < 50ms target; got {latency}ms");
        }
        Response::Err { error, .. } => panic!("unexpected error: {error:?}"),
    }
    let rows = state.audit.tail(10).await.unwrap();
    assert_eq!(rows[0].decision, homn_types::Decision::Allow);
}

#[tokio::test]
async fn us1_scenario_3_unmatched_call_falls_through_to_ask() {
    std::env::set_var("HOME", "/home/rsx");
    let (sock, state, _dir) = spawn_daemon().await;

    let req = decisions_create(
        "c",
        "WebFetch",
        json!({"url": "https://example.com"}),
        "/home/rsx/dev/x",
    );
    let resp = send(&sock, &req).await;
    match resp {
        Response::Ok { result, .. } => {
            assert_eq!(result["decision"], "ask");
            assert!(result["rule_source"].is_null());
        }
        Response::Err { error, .. } => panic!("unexpected error: {error:?}"),
    }
    let rows = state.audit.tail(10).await.unwrap();
    assert_eq!(rows[0].decision, homn_types::Decision::Ask);
}

#[tokio::test]
async fn many_decisions_in_a_row_all_persisted() {
    std::env::set_var("HOME", "/home/rsx");
    let (sock, state, _dir) = spawn_daemon().await;

    for i in 0..10 {
        let req = decisions_create(
            &format!("r{i}"),
            "Read",
            json!({"path": "/home/rsx/x"}),
            "/home/rsx",
        );
        let _ = send(&sock, &req).await;
    }
    let rows = state.audit.tail(20).await.unwrap();
    assert_eq!(rows.len(), 10);
}
