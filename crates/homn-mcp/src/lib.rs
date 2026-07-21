//! MCP server for `homn` (T073-T078).
//!
//! Exposes the daemon's policy + audit state as Model Context Protocol tools so an agent
//! (Claude Code, etc.) can introspect its own constraints:
//!
//! - `query_policy(tool, tool_input, cwd)` — dry-run evaluation. Returns the decision the
//!   engine *would* make for this call, the rule that would fire, and the rule's source
//!   location. **Does not log to audit, does not mutate state.**
//! - `explain_decision(decision_id)` — look up an audit row by id. Returns the rule that
//!   fired, the surface that answered (if any), the human's answer (if `ask`), and the
//!   end-to-end latency.
//! - `recent_decisions(limit, decision)` — tail the audit log; useful for the agent to ask
//!   *"what was just denied?"* and propose an alternative without re-attempting.
//!
//! The MCP server lives in the same process as the daemon (or can be spun up standalone via
//! `homn mcp stdio` for Claude's MCP config). It reads policy + audit through `ArcSwap` /
//! `tokio-rusqlite`, so hot reload + concurrent writes are transparent.
//!
//! See [ADR-0006](../../../docs/architecture/adr/0006-mcp-server.md) for why this is in the
//! design and why we expose it to the agent rather than hiding policy from it.

#![warn(missing_docs)]

mod brain;
mod rate_limit;

#[cfg(feature = "brain-agidb")]
pub use brain::AgidbBrain;
pub use brain::{Brain, MemoryBrain, RecallHit, RecordingBrain, TimelineEntry};
pub use rate_limit::{RateLimited, RateLimiter, DEFAULT_MAX_PER_WINDOW, DEFAULT_WINDOW};

use std::sync::Arc;
use std::time::Instant;

use homn_audit::Db;
use homn_policy::{Engine, EvalRequest, RuleSetHandle};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

/// Long-lived state the MCP tools read from. Equivalent in shape to homn-daemon's
/// `DaemonState`; kept here separately so this crate doesn't cyclically depend on the daemon.
#[derive(Clone)]
pub struct McpState {
    /// Policy engine.
    pub engine: Engine,
    /// Atomically-swappable ruleset (hot-reload aware).
    pub rules: RuleSetHandle,
    /// Audit DB.
    pub audit: Arc<Db>,
    /// The read-path memory for `recall` / `timeline` (v2). `None` when no brain is wired
    /// (e.g. the v1 policy-only server); the tools then return a clear "no brain" error.
    pub brain: Option<Arc<dyn Brain>>,
}

// ============================================================================
// Tool argument types
// ============================================================================

/// Args for `query_policy`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryPolicyArgs {
    /// The tool name (e.g. `"Bash"`, `"Read"`, `"WebFetch"`, `"mcp__server__tool"`).
    pub tool: String,
    /// The tool's input as JSON. The keys depend on the tool — `"command"` for Bash,
    /// `"path"` for Read/Edit/Write, `"url"` for WebFetch, etc.
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// Optional working directory; defaults to empty when omitted.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Args for `explain_decision`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExplainDecisionArgs {
    /// Audit-log id (as returned by `decisions.create` / `recent_decisions`).
    pub decision_id: i64,
}

/// Args for `recent_decisions`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecentDecisionsArgs {
    /// Maximum rows to return. Defaults to 10, capped at 100.
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Optional filter: `"allow"`, `"deny"`, or `"ask"`.
    #[serde(default)]
    pub decision: Option<String>,
    /// Optional filter: tool name.
    #[serde(default)]
    pub tool: Option<String>,
    /// Optional substring search across tool_input + tool_name + cwd (FTS5).
    #[serde(default)]
    pub grep: Option<String>,
}

fn default_limit() -> u32 {
    10
}

/// Args for `recall` (v2: US2). `as_of` is an optional ISO-8601 datetime for bi-temporal recall.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallArgs {
    /// The natural-language cue to recall against the local memory.
    pub cue: String,
    /// Optional bi-temporal bound: recall as the world was known at this ISO-8601 instant.
    pub as_of: Option<String>,
    /// Max hits to return. Defaults to 10, capped at 50.
    #[serde(default = "default_recall_k")]
    pub k: u32,
}

fn default_recall_k() -> u32 {
    10
}

/// Args for `timeline` (v2: US2).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TimelineArgs {
    /// The subject to trace: an entity name or topic phrase.
    pub subject: String,
    /// Inclusive start of the window (ISO-8601 datetime).
    pub from: String,
    /// Inclusive end of the window (ISO-8601 datetime).
    pub to: String,
}

fn parse_iso(s: &str) -> Result<chrono::DateTime<chrono::Utc>, McpError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| McpError::invalid_params(format!("invalid datetime `{s}`: {e}"), None))
}

// ============================================================================
// MCP server
// ============================================================================

/// The MCP server. Cheap to clone (state is all Arc-backed).
#[derive(Clone)]
pub struct HomnMcpServer {
    // Consumed by the `#[tool_handler]` macro at expand time; read via reflection in the
    // generated `call_tool` method. The compiler can't see that use directly, so silence the
    // dead-code warning here.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    state: McpState,
    /// Per-session quota — one server per stdio session, so one limiter is per-session (T079).
    rate_limiter: RateLimiter,
}

impl HomnMcpServer {
    /// Construct from shared state, using the default rate limit (100 calls / 60 s).
    pub fn new(state: McpState) -> Self {
        Self::with_rate_limiter(state, RateLimiter::with_defaults())
    }

    /// Construct with an explicit rate limiter. Used by tests to exercise the quota cheaply.
    pub fn with_rate_limiter(state: McpState, rate_limiter: RateLimiter) -> Self {
        Self {
            tool_router: Self::tool_router(),
            state,
            rate_limiter,
        }
    }

    /// Charge one call against the per-session quota, mapping exhaustion to an MCP error.
    fn enforce_rate_limit(&self) -> Result<(), McpError> {
        self.rate_limiter
            .check()
            .map_err(|limited| McpError::invalid_request(limited.to_string(), None))
    }
}

#[tool_router]
impl HomnMcpServer {
    #[tool(
        description = "Dry-run policy evaluation. Returns what decision homn would make for the given tool call WITHOUT logging or affecting any state. Use this before attempting an action you suspect may be denied — the response tells you the rule that would fire and lets you propose alternatives. Args: tool (string), tool_input (object), cwd (optional string)."
    )]
    async fn query_policy(
        &self,
        Parameters(args): Parameters<QueryPolicyArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let started = Instant::now();
        let cwd = args.cwd.clone().unwrap_or_default();
        let eval_req = EvalRequest::from_tool_call(&args.tool, &args.tool_input, &cwd);
        let rules = self.state.rules.load();
        let outcome = self.state.engine.eval(&rules, &eval_req);
        let latency_us = started.elapsed().as_micros();

        let body = serde_json::json!({
            "decision": decision_str(outcome.decision),
            "rule_source": outcome.rule.as_ref().map(|loc| serde_json::json!({
                "file": loc.file.display().to_string(),
                "line": loc.line,
            })),
            "rule_text": outcome.rule_text,
            "is_dry_run": true,
            "eval_latency_us": latency_us,
        });
        Ok(CallToolResult::success(vec![Content::json(body)?]))
    }

    #[tool(
        description = "Look up a single audit-log entry by its decision id. Returns the rule that fired, the surface that answered (if any), the human's answer (if it was ask-resolved), and timing. Use this to understand why a prior call was decided the way it was."
    )]
    async fn explain_decision(
        &self,
        Parameters(args): Parameters<ExplainDecisionArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        match find_decision_by_id(&self.state.audit, args.decision_id).await {
            Ok(Some(row)) => Ok(CallToolResult::success(vec![Content::json(
                record_to_json(&row),
            )?])),
            Ok(None) => Err(McpError::invalid_params(
                format!("no decision with id {}", args.decision_id),
                None,
            )),
            Err(err) => Err(McpError::internal_error(
                format!("audit lookup failed: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Return recent audit-log entries with optional filters. Useful for the agent to see what's been denied/allowed recently and propose alternatives. Args: limit (default 10, max 100), decision (\"allow\"/\"deny\"/\"ask\"), tool (filter by tool name), grep (FTS5 substring search)."
    )]
    async fn recent_decisions(
        &self,
        Parameters(args): Parameters<RecentDecisionsArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let limit = args.limit.clamp(1, 100);
        let decision = match args.decision.as_deref() {
            Some("allow") => Some(homn_types::Decision::Allow),
            Some("deny") => Some(homn_types::Decision::Deny),
            Some("ask") => Some(homn_types::Decision::Ask),
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("invalid decision filter `{other}` — must be allow|deny|ask"),
                    None,
                ));
            }
            None => None,
        };
        let query = homn_audit::LogQuery {
            decision,
            tool_name: args.tool.clone(),
            grep: args.grep.clone(),
            limit,
            ..Default::default()
        };
        match self.state.audit.query(query).await {
            Ok(rows) => {
                let arr: Vec<_> = rows.iter().map(record_to_json).collect();
                Ok(CallToolResult::success(vec![Content::json(arr)?]))
            }
            Err(err) => Err(McpError::internal_error(
                format!("audit query failed: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Recall from local memory. Returns ranked hits for a natural-language cue, each with provenance (source, app, captured_at, observation_id). Pure local math — no network call. Args: cue (string), as_of (optional ISO-8601 datetime — recall as the world was known then), k (optional, default 10)."
    )]
    async fn recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let brain =
            self.state.brain.as_ref().ok_or_else(|| {
                McpError::internal_error("no brain wired into this MCP server", None)
            })?;
        let as_of = match args.as_of.as_deref() {
            Some(s) => Some(parse_iso(s)?),
            None => None,
        };
        let k = args.k.clamp(1, 50) as usize;
        match brain.recall(&args.cue, as_of, k).await {
            Ok(hits) => Ok(CallToolResult::success(vec![Content::json(hits)?])),
            Err(err) => Err(McpError::internal_error(
                format!("recall failed: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Timeline of what happened for a subject (entity or topic) over a time window. Returns chronological entries with provenance. Pure local math — no network call. Args: subject (string), from (ISO-8601 datetime), to (ISO-8601 datetime)."
    )]
    async fn timeline(
        &self,
        Parameters(args): Parameters<TimelineArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let brain =
            self.state.brain.as_ref().ok_or_else(|| {
                McpError::internal_error("no brain wired into this MCP server", None)
            })?;
        let from = parse_iso(&args.from)?;
        let to = parse_iso(&args.to)?;
        if from > to {
            return Err(McpError::invalid_params(
                format!("`from` ({from}) must not be after `to` ({to})"),
                None,
            ));
        }
        match brain.timeline(&args.subject, from, to).await {
            Ok(entries) => Ok(CallToolResult::success(vec![Content::json(entries)?])),
            Err(err) => Err(McpError::internal_error(
                format!("timeline failed: {err}"),
                None,
            )),
        }
    }
}

fn decision_str(d: homn_types::Decision) -> &'static str {
    match d {
        homn_types::Decision::Allow => "allow",
        homn_types::Decision::Deny => "deny",
        homn_types::Decision::Ask => "ask",
    }
}

#[tool_handler]
impl ServerHandler for HomnMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "homn — the local human: memory, permissions, presence. \
             Use `recall` to fetch ranked memories for a cue, and `timeline` to trace a subject \
             over a window — both are local-only, every hit carries provenance. \
             Use `query_policy` to check what's allowed *before* attempting an action. \
             If a call was denied, use `explain_decision` to read the rule that fired. \
             Use `recent_decisions` to see what's happened recently.",
        )
    }
}

// ============================================================================
// Helpers
// ============================================================================

async fn find_decision_by_id(
    audit: &Arc<Db>,
    decision_id: i64,
) -> anyhow::Result<Option<homn_types::DecisionRecord>> {
    // We don't have a `by_id` query method; query a large window and filter.
    // For an MCP `explain_decision` call this is fine — the audit DB has at most ~30 days of data.
    let rows = audit
        .query(homn_audit::LogQuery {
            limit: 10_000,
            ..Default::default()
        })
        .await?;
    Ok(rows.into_iter().find(|r| r.id == decision_id))
}

fn record_to_json(rec: &homn_types::DecisionRecord) -> serde_json::Value {
    serde_json::json!({
        "decision_id": rec.id,
        "ts_millis": rec.ts_millis,
        "session_id": rec.session_id.0,
        "cwd": rec.cwd.display().to_string(),
        "tool_name": rec.tool_name,
        "tool_input": rec.tool_input,
        "decision": decision_str(rec.decision),
        "human_answer": rec.human_answer.map(|a| match a {
            homn_types::HumanAnswer::Allow => "allow",
            homn_types::HumanAnswer::Deny => "deny",
            homn_types::HumanAnswer::AlwaysAllow => "always_allow",
            homn_types::HumanAnswer::AlwaysDeny => "always_deny",
        }),
        "rule_source": rec.rule_source.as_ref().map(|loc| serde_json::json!({
            "file": loc.file.display().to_string(),
            "line": loc.line,
        })),
        "rule_text": rec.rule_text,
        "latency_ms": rec.latency_ms,
        "surface": rec.surface.map(|s| match s {
            homn_types::Surface::Tui => "tui",
            homn_types::Surface::Face => "face",
            homn_types::Surface::Ntfy => "ntfy",
            homn_types::Surface::Mcp => "mcp",
            homn_types::Surface::HookDirect => "hook-direct",
        }),
    })
}

// ============================================================================
// Stdio transport entry point
// ============================================================================

/// Run the MCP server on stdio. Blocks until the peer disconnects or an error occurs.
///
/// This is what `homn mcp stdio` calls. Claude Code spawns us, connects stdin↔stdout, and
/// drives the MCP protocol on top.
pub async fn serve_stdio(state: McpState) -> anyhow::Result<()> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;

    tracing::info!("homn mcp stdio server starting");
    let server = HomnMcpServer::new(state);
    let svc = server.serve(stdio()).await?;
    svc.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_swap::ArcSwap;
    use homn_policy::RuleSet;

    async fn test_state(policy_src: &str) -> McpState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, policy_src, "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: None,
        }
    }

    #[tokio::test]
    async fn tool_calls_are_rate_limited_and_report_a_clear_error() {
        // T079: the per-session quota is enforced at the tool boundary. With a quota of 2,
        // the third call in the window returns an MCP error naming the rate limit.
        use std::time::Duration;

        let state = test_state(r#"deny if tool == "Bash" && cmd.contains("rm -rf");"#).await;
        let server =
            HomnMcpServer::with_rate_limiter(state, RateLimiter::new(2, Duration::from_secs(60)));
        let call = || {
            server.query_policy(Parameters(QueryPolicyArgs {
                tool: "Bash".into(),
                tool_input: serde_json::json!({ "command": "ls" }),
                cwd: None,
            }))
        };

        assert!(call().await.is_ok(), "call 1 is under quota");
        assert!(call().await.is_ok(), "call 2 is under quota");

        let third = call().await;
        let err = third.expect_err("call 3 exceeds the quota of 2");
        assert!(
            err.message.to_lowercase().contains("rate limit"),
            "the error names the rate limit: {}",
            err.message,
        );
    }

    #[tokio::test]
    async fn server_constructs_with_state() {
        let state = test_state("").await;
        let server = HomnMcpServer::new(state);
        let info = server.get_info();
        assert!(
            info.instructions
                .as_ref()
                .is_some_and(|s| s.contains("homn")),
            "server info should mention homn"
        );
    }

    async fn state_with_brain(brain: Arc<dyn Brain>) -> McpState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: Some(brain),
        }
    }

    #[tokio::test]
    async fn recall_returns_provenance_hits_from_the_brain() {
        let brain = MemoryBrain::new();
        brain
            .push(homn_types::Observation {
                id: ulid::Ulid::new(),
                source: homn_types::SourceKind::Dictation,
                app: Some("Slack".into()),
                captured_at: chrono::Utc::now(),
                ingested_at: chrono::Utc::now(),
                text: "Sarah promised the quote by Friday".into(),
                redactions: vec![],
                session: None,
                speaker: None,
                content_hash: 0,
                provenance: homn_types::Provenance {
                    source_id: "test".into(),
                    upstream_ref: "t".into(),
                },
            })
            .await;
        let state = state_with_brain(Arc::new(brain)).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .recall(Parameters(RecallArgs {
                cue: "Sarah promised".into(),
                as_of: None,
                k: 5,
            }))
            .await
            .unwrap();
        // The result is JSON content; pull the hits back out.
        let body = res.content[0].as_text().unwrap().text.clone();
        let hits: Vec<RecallHit> = serde_json::from_str(&body).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "dictation");
        assert_eq!(hits[0].app.as_deref(), Some("Slack"));
        assert!(hits[0].observation_id.starts_with("01"));
    }

    #[tokio::test]
    async fn recall_without_a_brain_returns_a_clear_error() {
        let state = test_state("").await;
        let server = HomnMcpServer::new(state);
        let err = server
            .recall(Parameters(RecallArgs {
                cue: "x".into(),
                as_of: None,
                k: 3,
            }))
            .await
            .expect_err("no brain wired");
        assert!(
            err.message.to_lowercase().contains("no brain"),
            "{}",
            err.message
        );
    }

    #[tokio::test]
    async fn timeline_rejects_from_after_to() {
        let state = state_with_brain(Arc::new(MemoryBrain::new())).await;
        let server = HomnMcpServer::new(state);
        let err = server
            .timeline(Parameters(TimelineArgs {
                subject: "x".into(),
                from: "2026-07-20T00:00:00Z".into(),
                to: "2026-07-10T00:00:00Z".into(),
            }))
            .await
            .expect_err("from > to");
        assert!(
            err.message.to_lowercase().contains("must not be after"),
            "{}",
            err.message
        );
    }

    #[tokio::test]
    async fn recall_with_recording_brain_makes_no_other_io() {
        // SC-006 behavioural half: the handler touches only the brain. The RecordingBrain
        // records the call and returns empty; the structural half (tests/read_path_no_egress.rs)
        // forbids any HTTP-client dep that could egress.
        let brain = Arc::new(RecordingBrain::default());
        let state = state_with_brain(brain.clone()).await;
        let server = HomnMcpServer::new(state);
        let _ = server
            .recall(Parameters(RecallArgs {
                cue: "anything".into(),
                as_of: None,
                k: 3,
            }))
            .await
            .unwrap();
        let calls = brain.recall_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "recall hit the brain exactly once");
    }

    #[tokio::test]
    async fn timeline_calls_only_the_brain() {
        let brain = Arc::new(RecordingBrain::default());
        let state = state_with_brain(brain.clone()).await;
        let server = HomnMcpServer::new(state);
        let _ = server
            .timeline(Parameters(TimelineArgs {
                subject: "pricing".into(),
                from: "2026-07-01T00:00:00Z".into(),
                to: "2026-07-31T00:00:00Z".into(),
            }))
            .await
            .expect("timeline succeeds");
        let calls = brain.timeline_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "timeline hit the brain exactly once");
        assert_eq!(calls[0].0, "pricing");
    }

    #[tokio::test]
    async fn record_to_json_round_trips_known_fields() {
        let rec = homn_types::DecisionRecord {
            id: 7,
            ts_millis: 1_715_000_000_000,
            session_id: homn_types::SessionId::new("01HXY"),
            cwd: std::path::PathBuf::from("/home/x"),
            tool_name: "Bash".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            decision: homn_types::Decision::Deny,
            human_answer: None,
            rule_source: Some(homn_types::RuleSourceLocation {
                file: std::path::PathBuf::from("default.rhai"),
                line: 14,
            }),
            rule_text: Some("deny if ...".into()),
            ctxgraph_hit: None,
            latency_ms: 5,
            surface: Some(homn_types::Surface::HookDirect),
            source: homn_types::DecisionSource::Hook,
        };
        let v = record_to_json(&rec);
        assert_eq!(v["decision_id"], 7);
        assert_eq!(v["decision"], "deny");
        assert_eq!(v["rule_source"]["line"], 14);
        assert_eq!(v["surface"], "hook-direct");
    }

    #[test]
    fn decision_str_maps_all_variants() {
        assert_eq!(decision_str(homn_types::Decision::Allow), "allow");
        assert_eq!(decision_str(homn_types::Decision::Deny), "deny");
        assert_eq!(decision_str(homn_types::Decision::Ask), "ask");
    }
}
