//! Claude Code hook integration.
//!
//! This crate is the bridge between Claude Code's `~/.claude/settings.json` hook system and
//! `homn`'s Unix-socket RPC. Each `homn hook <event>` subcommand parses the hook payload from
//! stdin, calls the daemon over its socket, and writes the right hook-return JSON to stdout.
//!
//! See [`specs/001-policy-engine/contracts/hook-protocol.md`](../../../specs/001-policy-engine/contracts/hook-protocol.md)
//! for the wire format.
//!
//! ## Failure model — always safe
//!
//! The hook NEVER exits non-zero and NEVER blocks Claude. If anything fails (daemon down,
//! socket missing, malformed payload, timeout), the hook writes `{"behavior": "ask"}` and lets
//! Claude show its own interactive prompt. This is the "safe fallthrough" Constitution V demands.

// Note: `pty` module needs `unsafe` for the TIOCGWINSZ ioctl call. We confine the unsafe to
// that file and `#![forbid(unsafe_code)]` everywhere else via per-module attributes.
#![warn(missing_docs)]

pub mod install;
pub mod pty;
pub mod setup;

pub use install::{default_settings_path, install_snippet, run_install, InstallReport};
pub use pty::{run_under_pty, PtyConfig, PtyExit};
pub use setup::{
    detect_init_system, launchd_plist, run_setup, run_uninstall, seed_policy, systemd_unit,
    InitSystem, PolicyProfile, PolicySeedOutcome, ServiceOutcome, SetupOptions, SetupReport,
    UninstallReport,
};

use std::path::{Path, PathBuf};
use std::time::Duration;

use homn_tui::PromptPayload;
use homn_types::{HumanAnswer, Request, Response, RuleSourceLocation};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Time budget for one round-trip to the daemon. Under Claude Code's 30s hook timeout.
pub const DAEMON_TIMEOUT: Duration = Duration::from_secs(28);

/// Format a "safe fallthrough" PermissionRequest response that tells Claude to fall back to
/// its own interactive prompt. Used whenever anything goes wrong.
pub fn safe_fallthrough_response() -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": "ask" }
        }
    })
}

/// Construct the PermissionRequest hook return for a given daemon decision.
pub fn permission_request_response(behavior: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": behavior }
        }
    })
}

/// The fields we extract from Claude Code's `PermissionRequest` hook payload.
#[derive(Debug, Deserialize)]
struct HookPayload {
    #[serde(default)]
    session_id: String,
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
    /// Some Claude versions put `cwd` at the top of the payload; others inside `tool_input`.
    /// We accept both.
    #[serde(default)]
    cwd: Option<String>,
}

/// Handle a `PermissionRequest` hook end-to-end: parse stdin, call the daemon, return the wire
/// response. Errors produce the safe-fallthrough response, NOT a hook failure.
pub async fn handle_permission_request(socket_path: impl AsRef<Path>, stdin_json: &str) -> Value {
    match handle_permission_request_inner(socket_path.as_ref(), stdin_json).await {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(error = %err, "hook degrading to safe fallthrough");
            safe_fallthrough_response()
        }
    }
}

async fn handle_permission_request_inner(
    socket_path: &Path,
    stdin_json: &str,
) -> anyhow::Result<Value> {
    let payload: HookPayload = serde_json::from_str(stdin_json)?;
    let cwd = payload
        .cwd
        .clone()
        .or_else(|| {
            payload
                .tool_input
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
        })
        .unwrap_or_default();

    let request = Request {
        id: ulid::Ulid::new().to_string(),
        method: "decisions.create".into(),
        params: json!({
            "source": "hook",
            "session_id": payload.session_id,
            "cwd": cwd,
            "tool_name": payload.tool_name,
            "tool_input": payload.tool_input,
        }),
    };

    let response = timeout(DAEMON_TIMEOUT, call_daemon(socket_path, &request)).await??;

    let result = match response {
        Response::Ok { result, .. } => result,
        Response::Err { error, .. } => {
            tracing::warn!(?error, "daemon returned error; falling through to ask");
            return Ok(permission_request_response("ask"));
        }
    };

    let decision = result
        .get("decision")
        .and_then(|v| v.as_str())
        .unwrap_or("ask");

    // Deterministic decisions short-circuit. Only `ask` triggers the TUI round-trip.
    if decision != "ask" {
        return Ok(permission_request_response(decision));
    }

    // ===== Ask path (T031-T033) =====
    let decision_id = result
        .get("decision_id")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let prompt_payload = build_prompt_payload(&payload, &cwd, decision_id, &result);

    // Open the TUI prompt on /dev/tty. Runs on a blocking thread because it does sync file I/O.
    let answer: Option<HumanAnswer> =
        tokio::task::spawn_blocking(move || homn_tui::prompt_user(&prompt_payload))
            .await
            .unwrap_or(None);

    // Post resolution back to the daemon so the audit row reflects the human's answer.
    let resolve_req = Request {
        id: ulid::Ulid::new().to_string(),
        method: "decisions.resolve".into(),
        params: json!({
            "decision_id": decision_id,
            "human_answer": answer.map(human_answer_str),
            "surface": "tui",
        }),
    };
    if let Err(err) = call_daemon(socket_path, &resolve_req).await {
        tracing::warn!(error = %err, "decisions.resolve failed; continuing");
    }

    // Map the user's answer back to a hook behavior. None = defer = let claude prompt.
    let behavior = match answer {
        Some(HumanAnswer::Allow) | Some(HumanAnswer::AlwaysAllow) => "allow",
        Some(HumanAnswer::Deny) | Some(HumanAnswer::AlwaysDeny) => "deny",
        None => "ask",
    };
    Ok(permission_request_response(behavior))
}

fn build_prompt_payload(
    payload: &HookPayload,
    cwd: &str,
    decision_id: i64,
    result: &Value,
) -> PromptPayload {
    let preview = preview_tool_input(&payload.tool_input);
    let rule_source = result
        .get("rule_source")
        .filter(|v| !v.is_null())
        .and_then(|v| {
            let file = v.get("file").and_then(|x| x.as_str())?;
            let line = v.get("line").and_then(|x| x.as_u64())? as u32;
            Some(RuleSourceLocation {
                file: PathBuf::from(file),
                line,
            })
        });
    let rule_text = result
        .get("rule_text")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    PromptPayload {
        decision_id,
        session_id: payload.session_id.clone(),
        tool_name: payload.tool_name.clone(),
        tool_input_preview: preview,
        cwd: PathBuf::from(cwd),
        rule_source,
        rule_text,
    }
}

fn preview_tool_input(v: &Value) -> String {
    if let Some(cmd) = v.get("command").and_then(|x| x.as_str()) {
        return clip(cmd, 100);
    }
    if let Some(path) = v.get("path").and_then(|x| x.as_str()) {
        return clip(path, 100);
    }
    if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
        return clip(url, 100);
    }
    clip(&v.to_string(), 100)
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn human_answer_str(a: HumanAnswer) -> &'static str {
    match a {
        HumanAnswer::Allow => "allow",
        HumanAnswer::Deny => "deny",
        HumanAnswer::AlwaysAllow => "always_allow",
        HumanAnswer::AlwaysDeny => "always_deny",
    }
}

async fn call_daemon(socket_path: &Path, request: &Request) -> anyhow::Result<Response> {
    let stream = UnixStream::connect(socket_path).await?;
    let (read_half, mut write_half) = stream.into_split();
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    write_half.write_all(line.as_bytes()).await?;
    write_half.flush().await?;
    write_half.shutdown().await?;

    let mut reader = BufReader::new(read_half).lines();
    let resp_line = reader
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("daemon closed without responding"))?;
    let response: Response = serde_json::from_str(&resp_line)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn safe_fallthrough_has_required_shape() {
        let v = safe_fallthrough_response();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "ask");
    }

    #[test]
    fn permission_request_response_shape() {
        let v = permission_request_response("deny");
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    }

    #[tokio::test]
    async fn handle_permission_request_falls_through_when_daemon_missing() {
        // Point at a socket path that doesn't exist; expect safe fallthrough rather than panic.
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("missing.sock");
        let body = json!({
            "session_id": "01H",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"}
        })
        .to_string();
        let resp = handle_permission_request(&nonexistent, &body).await;
        assert_eq!(resp["hookSpecificOutput"]["decision"]["behavior"], "ask");
    }

    #[tokio::test]
    async fn handle_permission_request_falls_through_on_malformed_input() {
        let dir = tempfile::tempdir().unwrap();
        let resp = handle_permission_request(dir.path().join("x.sock"), "not json").await;
        assert_eq!(resp["hookSpecificOutput"]["decision"]["behavior"], "ask");
    }
}
