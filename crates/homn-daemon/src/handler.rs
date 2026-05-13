//! Daemon-side request dispatch.
//!
//! Maps JSON-line RPC `Request`s to `Response`s by consulting the policy engine and writing to
//! the audit log. T030 ships the deterministic path (`decisions.create` for `Allow` / `Deny` /
//! `Ask` without human interaction); the `Ask` round-trip via face/TUI lands in T032/T033.

use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use homn_audit::{Db, NewDecision};
use homn_policy::{Engine, EvalRequest, Outcome, RuleSet, RuleSetHandle};
use homn_types::{Decision, DecisionSource, ErrorObject, Request, Response, SessionId, Surface};
use serde::Deserialize;
use serde_json::json;

/// Long-lived state shared between connection handlers.
#[derive(Clone)]
pub struct DaemonState {
    /// Policy engine (sandboxed Rhai with our custom helpers).
    pub engine: Engine,
    /// Compiled ruleset. Backed by `ArcSwap` so a watcher task (T026) can swap it atomically
    /// from another thread without blocking the request hot path.
    pub rules: RuleSetHandle,
    /// Audit DB handle.
    pub audit: Arc<Db>,
}

impl DaemonState {
    /// Construct a DaemonState with a static (non-reloading) ruleset. Used in tests and as the
    /// starting point in production before the watcher kicks in.
    pub fn with_static_rules(engine: Engine, rules: RuleSet, audit: Arc<Db>) -> Self {
        Self {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
        }
    }
}

/// Parameters for `decisions.create`. Matches `specs/.../contracts/hook-protocol.md`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateDecisionParams {
    /// Where the request originated (`hook` | `pty-wrapper` | `mcp`).
    source: DecisionSource,
    /// Claude Code session ULID.
    session_id: String,
    /// Working directory of the calling session.
    cwd: String,
    /// Tool name.
    tool_name: String,
    /// Tool input as JSON.
    #[serde(default)]
    tool_input: serde_json::Value,
}

/// Dispatch a single RPC request. Returns a [`Response`] suitable for writing back to the wire.
pub async fn dispatch(state: &DaemonState, req: Request) -> Response {
    match req.method.as_str() {
        "ping" => Response::ok(req.id, json!({"pong": true})),
        "decisions.create" => handle_decisions_create(state, req).await,
        other => Response::err(
            req.id,
            ErrorObject::new(
                "unknown_method",
                format!("method `{other}` is not implemented yet"),
            ),
        ),
    }
}

async fn handle_decisions_create(state: &DaemonState, req: Request) -> Response {
    let params: CreateDecisionParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(err) => {
            return Response::err(req.id, ErrorObject::new("invalid_params", err.to_string()));
        }
    };

    let started = Instant::now();
    let eval_req = EvalRequest::from_tool_call(&params.tool_name, &params.tool_input, &params.cwd);
    let rules = state.rules.load();
    let outcome: Outcome = state.engine.eval(&rules, &eval_req);
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

    // For T030 we write deterministic decisions (Allow / Deny) and synthesized Ask straight
    // through. The human-interactive Ask round-trip is wired in T032/T033.
    let surface = match outcome.decision {
        Decision::Allow | Decision::Deny => Some(Surface::HookDirect),
        Decision::Ask => None,
    };

    let new_decision = NewDecision {
        ts_millis: unix_millis_now(),
        session_id: SessionId::new(params.session_id),
        cwd: params.cwd,
        tool_name: params.tool_name,
        tool_input: params.tool_input,
        decision: outcome.decision,
        human_answer: None,
        rule_source: outcome.rule.clone(),
        rule_text: outcome.rule_text.clone(),
        ctxgraph_hit: None,
        latency_ms,
        surface,
        source: params.source,
    };

    let decision_id = match state.audit.write_decision(new_decision).await {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(error = %err, "failed to write audit row");
            return Response::err(
                req.id,
                ErrorObject::new("audit_write_failed", err.to_string()),
            );
        }
    };

    let body = json!({
        "decision_id": decision_id,
        "decision": decision_as_str(outcome.decision),
        "rule_source": outcome.rule.as_ref().map(|loc| json!({
            "file": loc.file.display().to_string(),
            "line": loc.line,
        })),
        "rule_text": outcome.rule_text,
        "context": serde_json::Value::Null,
        "latency_ms": latency_ms,
    });

    Response::ok(req.id, body)
}

fn decision_as_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
        Decision::Ask => "ask",
    }
}

fn unix_millis_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_policy::{Engine, RuleSet};
    use serde_json::json;

    async fn state_with_rules(src: &str) -> DaemonState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, src, "default.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        DaemonState::with_static_rules(engine, rules, audit)
    }

    fn dc_request(id: &str, tool: &str, tool_input: serde_json::Value) -> Request {
        Request {
            id: id.into(),
            method: "decisions.create".into(),
            params: json!({
                "source": "hook",
                "session_id": "01HXY",
                "cwd": "/home/rsx/dev/x",
                "tool_name": tool,
                "tool_input": tool_input,
            }),
        }
    }

    #[tokio::test]
    async fn deny_rule_path_writes_audit_row() {
        let state = state_with_rules(r#"deny if tool == "Bash" && cmd.contains("rm -rf");"#).await;
        let resp = dispatch(
            &state,
            dc_request("a", "Bash", json!({"command": "rm -rf ~/x"})),
        )
        .await;
        match resp {
            Response::Ok { id, result } => {
                assert_eq!(id, "a");
                assert_eq!(result["decision"], "deny");
                assert_eq!(result["rule_source"]["line"], 1);
            }
            Response::Err { error, .. } => panic!("expected Ok, got error: {error:?}"),
        }
        let rows = state.audit.tail(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decision, Decision::Deny);
    }

    #[tokio::test]
    async fn allow_rule_path_writes_audit_row() {
        let state = state_with_rules(r#"allow if tool == "Read";"#).await;
        let resp = dispatch(
            &state,
            dc_request("b", "Read", json!({"path": "/home/rsx/foo"})),
        )
        .await;
        match resp {
            Response::Ok { result, .. } => {
                assert_eq!(result["decision"], "allow");
            }
            Response::Err { error, .. } => panic!("unexpected error: {error:?}"),
        }
        let rows = state.audit.tail(10).await.unwrap();
        assert_eq!(rows[0].decision, Decision::Allow);
    }

    #[tokio::test]
    async fn unmatched_request_falls_through_to_ask() {
        let state = state_with_rules(r#"allow if tool == "Read";"#).await;
        let resp = dispatch(
            &state,
            dc_request("c", "WebFetch", json!({"url": "https://x"})),
        )
        .await;
        match resp {
            Response::Ok { result, .. } => {
                assert_eq!(result["decision"], "ask");
                assert!(result["rule_source"].is_null());
            }
            Response::Err { error, .. } => panic!("unexpected error: {error:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_params_returns_invalid_params_error() {
        let state = state_with_rules("").await;
        let req = Request {
            id: "d".into(),
            method: "decisions.create".into(),
            // missing source / session_id / tool_name / cwd
            params: json!({"tool_name": "Bash"}),
        };
        let resp = dispatch(&state, req).await;
        match resp {
            Response::Err { id, error } => {
                assert_eq!(id, "d");
                assert_eq!(error.code, "invalid_params");
            }
            Response::Ok { .. } => panic!("expected invalid_params error"),
        }
    }

    #[tokio::test]
    async fn ping_still_works_via_dispatch() {
        let state = state_with_rules("").await;
        let resp = dispatch(
            &state,
            Request {
                id: "p".into(),
                method: "ping".into(),
                params: json!({}),
            },
        )
        .await;
        match resp {
            Response::Ok { result, .. } => assert_eq!(result["pong"], true),
            Response::Err { error, .. } => panic!("expected Ok, got {error:?}"),
        }
    }
}
