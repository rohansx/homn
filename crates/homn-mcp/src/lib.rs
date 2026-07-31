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

mod beliefs;
mod brain;
mod commitments;
mod forget;
mod rate_limit;

pub use beliefs::{Beliefs, MemoryBeliefs};
#[cfg(feature = "brain-agidb")]
pub use brain::AgidbBrain;
pub use brain::{Brain, MemoryBrain, RecallHit, RecordingBrain, TimelineEntry};
pub use commitments::{Commitments, MemoryCommitments};
pub use forget::{Forget, MemoryForget};
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
    /// The commitment store for `commitments(status?, due_before?)` (v2 US5). `None` when no
    /// commitment source is wired; the tool then returns a clear "no commitments" error.
    pub commitments: Option<Arc<dyn Commitments>>,
    /// The belief store for `beliefs(topic)` (v2 US5/US6). `None` when no belief source is
    /// wired; the tool then returns a clear "no beliefs" error.
    pub beliefs: Option<Arc<dyn Beliefs>>,
    /// The unlearn handle for `forget(scope)` (v2 US4) — the one write tool. `None` when no
    /// forget source is wired; the tool then returns a clear "no forget wired" error.
    pub forget: Option<Arc<dyn Forget>>,
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

/// Args for `commitments` (v2: US5). Both filters optional.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CommitmentsArgs {
    /// Filter by status: `"open"`, `"fulfilled"`, `"overdue"`, `"cancelled"`. Omit for any.
    #[serde(default)]
    pub status: Option<String>,
    /// Only commitments due on/before this ISO-8601 datetime. Omit for any due date.
    #[serde(default)]
    pub due_before: Option<String>,
}

/// Args for `beliefs` (v2: US5/US6).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BeliefsArgs {
    /// The topic to look up (substring match; empty = all topics).
    pub topic: String,
    /// If true, return only the current (non-superseded) position per topic, dropping history.
    #[serde(default)]
    pub current_only: bool,
}

/// Args for `whodis` (v2: US6) — a relationship dossier for a named person.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WhodisArgs {
    /// The person's name (e.g. "Sarah", "Marco").
    pub name: String,
}

/// Args for `today` / `standup` (v2: US6) — a daily recap.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TodayArgs {
    /// The date to recap, as `YYYY-MM-DD` (defaults to today UTC when omitted).
    #[serde(default)]
    pub date: Option<String>,
}

/// Args for `forget` (v2: US4) — the one write tool. Exactly one of entity/pattern/from+to.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ForgetArgs {
    /// Forget everything about a named entity.
    #[serde(default)]
    pub entity: Option<String>,
    /// Forget everything matching a pattern.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Time-range start (ISO-8601). Use with `to`.
    #[serde(default)]
    pub from: Option<String>,
    /// Time-range end (ISO-8601). Use with `from`.
    #[serde(default)]
    pub to: Option<String>,
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

    #[tool(
        description = "List extracted commitments (promises, mine and theirs). Returns each with text, owner, counterpart, due, status, source_obs, extracted_by — pure local query, no network. Args: status (optional: open|fulfilled|overdue|cancelled), due_before (optional ISO-8601 datetime)."
    )]
    async fn commitments(
        &self,
        Parameters(args): Parameters<CommitmentsArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let store = self.state.commitments.as_ref().ok_or_else(|| {
            McpError::internal_error("no commitment store wired into this MCP server", None)
        })?;
        let status = match args.status.as_deref() {
            Some("open") => Some(homn_types::CommitmentStatus::Open),
            Some("fulfilled") => Some(homn_types::CommitmentStatus::Fulfilled),
            Some("overdue") => Some(homn_types::CommitmentStatus::Overdue),
            Some("cancelled") => Some(homn_types::CommitmentStatus::Cancelled),
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("invalid status `{other}` — must be open|fulfilled|overdue|cancelled"),
                    None,
                ));
            }
            None => None,
        };
        let due_before = match args.due_before.as_deref() {
            Some(s) => Some(parse_iso(s)?),
            None => None,
        };
        match store.commitments(status, due_before).await {
            Ok(cs) => Ok(CallToolResult::success(vec![Content::json(cs)?])),
            Err(err) => Err(McpError::internal_error(
                format!("commitments query failed: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Look up beliefs (positions taken over time) for a topic. Returns the current position plus its revision history, each with position, confidence, valid_from, superseded_at, source_obs. Pure local query, no network. Args: topic (substring match), current_only (optional: drop the revision history)."
    )]
    async fn beliefs(
        &self,
        Parameters(args): Parameters<BeliefsArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let store = self.state.beliefs.as_ref().ok_or_else(|| {
            McpError::internal_error("no belief store wired into this MCP server", None)
        })?;
        match store.beliefs(&args.topic, args.current_only).await {
            Ok(bs) => Ok(CallToolResult::success(vec![Content::json(bs)?])),
            Err(err) => Err(McpError::internal_error(
                format!("beliefs query failed: {err}"),
                None,
            )),
        }
    }

    #[tool(
        description = "Relationship dossier for a person: interactions count, last thread, and open loops (commitments owed to/from them). Aggregates recall + commitments — pure local, no network. Args: name (the person)."
    )]
    async fn whodis(
        &self,
        Parameters(args): Parameters<WhodisArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let brain = self.state.brain.as_ref().ok_or_else(|| {
            McpError::internal_error("no brain wired — whodis needs recall", None)
        })?;
        // Interactions = recall hits that mention the name; last_thread = the most recent.
        let hits = brain
            .recall(&args.name, None, 50)
            .await
            .map_err(|e| McpError::internal_error(format!("recall failed: {e}"), None))?;
        let interactions_count = hits.len();
        let last_thread = hits.iter().max_by_key(|h| h.captured_at).cloned();
        // Open loops = open commitments where this person is the counterpart.
        let mut open_loops = Vec::new();
        if let Some(cs) = self.state.commitments.as_ref() {
            let all = cs
                .commitments(Some(homn_types::CommitmentStatus::Open), None)
                .await
                .map_err(|e| McpError::internal_error(format!("commitments failed: {e}"), None))?;
            open_loops = all
                .into_iter()
                .filter(|c| {
                    let owner_is = matches!(&c.owner, homn_types::Party::Entity(n) if n.eq_ignore_ascii_case(&args.name));
                    let counterpart_is = matches!(&c.counterpart, Some(homn_types::Party::Entity(n)) if n.eq_ignore_ascii_case(&args.name));
                    owner_is || counterpart_is
                })
                .map(|c| serde_json::json!({
                    "text": c.text,
                    "due": c.due.map(|d| d.to_rfc3339()),
                    "owner": c.owner,
                }))
                .collect();
        }
        let body = serde_json::json!({
            "entity": args.name,
            "interactions_count": interactions_count,
            "last_thread": last_thread,
            "open_loops": open_loops,
        });
        Ok(CallToolResult::success(vec![Content::json(body)?]))
    }

    #[tool(
        description = "Daily recap: what you did today (timeline), plus commitments touched (due or created today). Aggregates timeline + commitments — pure local, no network. Args: date (optional YYYY-MM-DD, defaults to today UTC)."
    )]
    async fn today(
        &self,
        Parameters(args): Parameters<TodayArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let brain = self.state.brain.as_ref().ok_or_else(|| {
            McpError::internal_error("no brain wired — today needs timeline", None)
        })?;
        // Resolve the day (UTC), [00:00, 24:00).
        let day = match args.date.as_deref() {
            Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
                McpError::invalid_params(
                    format!("invalid date `{s}` (expect YYYY-MM-DD): {e}"),
                    None,
                )
            })?,
            None => chrono::Utc::now().date_naive(),
        };
        let from = day.and_hms_opt(0, 0, 0).unwrap().and_utc();
        let to = (day + chrono::Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        // "did" = the day's timeline entries (sessions are the entries themselves).
        let did = brain
            .timeline("", from, to)
            .await
            .map_err(|e| McpError::internal_error(format!("timeline failed: {e}"), None))?;
        let sessions: Vec<_> = did
            .iter()
            .map(|e| serde_json::json!({ "at": e.at.to_rfc3339(), "text": e.text, "source": e.source }))
            .collect();
        // Commitments touched = open commitments due on/before end-of-day (due today or overdue).
        let mut commitments_touched = Vec::new();
        if let Some(cs) = self.state.commitments.as_ref() {
            let due = cs
                .commitments(Some(homn_types::CommitmentStatus::Open), Some(to))
                .await
                .map_err(|e| McpError::internal_error(format!("commitments failed: {e}"), None))?;
            commitments_touched = due
                .into_iter()
                .map(|c| serde_json::json!({ "text": c.text, "due": c.due.map(|d| d.to_rfc3339()), "owner": c.owner }))
                .collect();
        }
        let body = serde_json::json!({
            "date": day.to_string(),
            "sessions": sessions,
            "did_count": did.len(),
            "commitments_touched": commitments_touched,
        });
        Ok(CallToolResult::success(vec![Content::json(body)?]))
    }

    #[tool(
        description = "Forget (unlearn) memories matching a scope — the one write tool. After success the matched memory stops surfacing in the other tools. Returns a receipt_id + match_count + scope (proves the scope without re-exposing content). Args: entity (name) OR pattern OR from+to (ISO-8601 window)."
    )]
    async fn forget(
        &self,
        Parameters(args): Parameters<ForgetArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.enforce_rate_limit()?;
        let forget = self.state.forget.as_ref().ok_or_else(|| {
            McpError::internal_error("no forget handle wired into this MCP server", None)
        })?;
        let scope = if let Some(name) = args.entity {
            homn_types::ForgetScope::Entity(name)
        } else if let Some(p) = args.pattern {
            homn_types::ForgetScope::Pattern(p)
        } else if let (Some(f), Some(t)) = (args.from.as_ref(), args.to.as_ref()) {
            let from = parse_iso(f)?;
            let to = parse_iso(t)?;
            if from > to {
                return Err(McpError::invalid_params(
                    format!("`from` ({from}) must not be after `to` ({to})"),
                    None,
                ));
            }
            homn_types::ForgetScope::TimeRange { from, to }
        } else {
            return Err(McpError::invalid_params(
                "specify a scope: entity, pattern, or from+to",
                None,
            ));
        };
        let receipt = forget
            .forget(&scope)
            .await
            .map_err(|e| McpError::internal_error(format!("forget failed: {e}"), None))?;
        let body = serde_json::json!({
            "receipt_id": format!("del-{}", receipt.match_count), // opaque id; the ledger seq is added by the daemon
            "scope": receipt.scope,
            "match_count": receipt.match_count,
            "at": receipt.at.to_rfc3339(),
        });
        Ok(CallToolResult::success(vec![Content::json(body)?]))
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
            commitments: None,
            beliefs: None,
            forget: None,
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
            commitments: None,
            beliefs: None,
            forget: None,
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

    async fn state_with_commitments(c: Arc<dyn Commitments>) -> McpState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: None,
            commitments: Some(c),
            beliefs: None,
            forget: None,
        }
    }

    #[tokio::test]
    async fn commitments_tool_returns_the_store_contents() {
        use homn_types::{CommitmentId, ExtractionSource, Party};
        let store = Arc::new(MemoryCommitments::new());
        store.push(homn_types::Commitment {
            id: CommitmentId::new(),
            text: "I'll send the pricing quote by Friday".into(),
            owner: Party::Me,
            counterpart: Some(Party::Entity("Marco".into())),
            due: None,
            status: homn_types::CommitmentStatus::Open,
            source_obs: "obs-1".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: chrono::Utc::now(),
        });
        let state = state_with_commitments(store.clone()).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .commitments(Parameters(CommitmentsArgs {
                status: None,
                due_before: None,
            }))
            .await
            .unwrap();
        let body = res.content[0].as_text().unwrap().text.clone();
        let cs: Vec<homn_types::Commitment> = serde_json::from_str(&body).unwrap();
        assert_eq!(cs.len(), 1);
        assert!(cs[0].text.contains("pricing quote"));
        assert!(matches!(&cs[0].counterpart, Some(homn_types::Party::Entity(n)) if n == "Marco"));
    }

    #[tokio::test]
    async fn commitments_tool_filters_by_status() {
        use homn_types::{CommitmentId, ExtractionSource, Party};
        let store = Arc::new(MemoryCommitments::new());
        for (txt, status) in [
            ("open one", homn_types::CommitmentStatus::Open),
            ("done one", homn_types::CommitmentStatus::Fulfilled),
        ] {
            store.push(homn_types::Commitment {
                id: CommitmentId::new(),
                text: txt.into(),
                owner: Party::Me,
                counterpart: None,
                due: None,
                status,
                source_obs: "obs".into(),
                extracted_by: ExtractionSource::Regex,
                disclosure_receipt: None,
                created_at: chrono::Utc::now(),
            });
        }
        let state = state_with_commitments(store).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .commitments(Parameters(CommitmentsArgs {
                status: Some("fulfilled".into()),
                due_before: None,
            }))
            .await
            .unwrap();
        let cs: Vec<homn_types::Commitment> =
            serde_json::from_str(&res.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "done one");
    }

    #[tokio::test]
    async fn commitments_without_a_store_returns_a_clear_error() {
        let state = test_state("").await;
        let server = HomnMcpServer::new(state);
        let err = server
            .commitments(Parameters(CommitmentsArgs {
                status: None,
                due_before: None,
            }))
            .await
            .expect_err("no commitments wired");
        assert!(
            err.message.to_lowercase().contains("no commitment"),
            "{}",
            err.message
        );
    }

    async fn state_with_beliefs(b: Arc<dyn Beliefs>) -> McpState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: None,
            commitments: None,
            beliefs: Some(b),
            forget: None,
        }
    }

    #[tokio::test]
    async fn beliefs_tool_returns_current_and_history() {
        use homn_types::{BeliefId, ExtractionSource};
        let store = Arc::new(MemoryBeliefs::new());
        for (position, current) in [("use agidb", false), ("actually ctxgraph", true)] {
            store.push(homn_types::Belief {
                id: BeliefId::new(),
                topic: "the brain".into(),
                position: position.into(),
                confidence: 1.0,
                valid_from: chrono::Utc::now(),
                superseded_at: if current {
                    None
                } else {
                    Some(chrono::Utc::now())
                },
                source_obs: "obs".into(),
                extracted_by: ExtractionSource::Regex,
                disclosure_receipt: None,
                created_at: chrono::Utc::now(),
            });
        }
        let state = state_with_beliefs(store).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .beliefs(Parameters(BeliefsArgs {
                topic: "brain".into(),
                current_only: false,
            }))
            .await
            .unwrap();
        let bs: Vec<homn_types::Belief> =
            serde_json::from_str(&res.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(bs.len(), 2, "current + history");
        assert!(bs[0].superseded_at.is_none(), "current first");
        assert!(bs[0].position.contains("ctxgraph"));
    }

    #[tokio::test]
    async fn beliefs_without_a_store_returns_a_clear_error() {
        let state = test_state("").await;
        let server = HomnMcpServer::new(state);
        let err = server
            .beliefs(Parameters(BeliefsArgs {
                topic: "x".into(),
                current_only: false,
            }))
            .await
            .expect_err("no beliefs wired");
        assert!(
            err.message.to_lowercase().contains("no belief"),
            "{}",
            err.message
        );
    }

    async fn state_with_brain_and_commitments(
        brain: Arc<dyn Brain>,
        commitments: Arc<dyn Commitments>,
    ) -> McpState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: Some(brain),
            commitments: Some(commitments),
            beliefs: None,
            forget: None,
        }
    }

    #[tokio::test]
    async fn whodis_aggregates_interactions_and_open_loops() {
        use homn_types::{CommitmentId, ExtractionSource, Party};
        // Brain with one interaction mentioning Sarah.
        let brain = Arc::new(MemoryBrain::new());
        brain
            .push(homn_types::Observation {
                id: ulid::Ulid::new(),
                source: homn_types::SourceKind::ScreenOcr,
                app: Some("Slack".into()),
                captured_at: chrono::Utc::now(),
                ingested_at: chrono::Utc::now(),
                text: "Sarah promised the user research by Wednesday".into(),
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
        // Commitments: one open loop owed to Sarah.
        let cs = Arc::new(MemoryCommitments::new());
        cs.push(homn_types::Commitment {
            id: CommitmentId::new(),
            text: "user research by Wednesday".into(),
            owner: Party::Entity("Sarah".into()),
            counterpart: Some(Party::Me),
            due: None,
            status: homn_types::CommitmentStatus::Open,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: chrono::Utc::now(),
        });
        let state = state_with_brain_and_commitments(brain, cs).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .whodis(Parameters(WhodisArgs {
                name: "Sarah".into(),
            }))
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&res.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(body["entity"], "Sarah");
        assert!(
            body["interactions_count"].as_u64().unwrap() >= 1,
            "Sarah is recalled"
        );
        assert!(
            !body["open_loops"].as_array().unwrap().is_empty(),
            "an open loop with Sarah"
        );
    }

    #[tokio::test]
    async fn today_returns_the_days_timeline_and_commitments_due() {
        // Brain with a timeline entry today.
        let brain = Arc::new(MemoryBrain::new());
        let now = chrono::Utc::now();
        brain
            .push(homn_types::Observation {
                id: ulid::Ulid::new(),
                source: homn_types::SourceKind::ScreenOcr,
                app: Some("VS Code".into()),
                captured_at: now,
                ingested_at: now,
                text: "wired the whodis tool".into(),
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
        let cs = Arc::new(MemoryCommitments::new());
        let state = state_with_brain_and_commitments(brain, cs).await;
        let server = HomnMcpServer::new(state);
        let res = server
            .today(Parameters(TodayArgs { date: None }))
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&res.content[0].as_text().unwrap().text).unwrap();
        assert!(
            body["did_count"].as_u64().unwrap() >= 1,
            "today's timeline has the entry"
        );
        assert!(body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["text"] == "wired the whodis tool"));
    }

    #[tokio::test]
    async fn whodis_without_a_brain_returns_a_clear_error() {
        let state = test_state("").await;
        let server = HomnMcpServer::new(state);
        let err = server
            .whodis(Parameters(WhodisArgs { name: "x".into() }))
            .await
            .expect_err("no brain");
        assert!(
            err.message.to_lowercase().contains("no brain"),
            "{}",
            err.message
        );
    }

    /// T057: the seven-tool MCP surface answers its query types end-to-end with provenance, in
    /// one pass. Populates brain + commitments + beliefs + forget, then drives recall, timeline,
    /// commitments, beliefs, whodis, today, forget in sequence.
    #[tokio::test]
    async fn seven_tools_answer_with_provenance_in_one_pass() {
        use homn_types::{BeliefId, CommitmentId, ExtractionSource, ForgetScope, Party};

        // --- populate ---
        let brain = Arc::new(MemoryBrain::new());
        let now = chrono::Utc::now();
        brain
            .push(homn_types::Observation {
                id: ulid::Ulid::new(),
                source: homn_types::SourceKind::ScreenOcr,
                app: Some("Slack".into()),
                captured_at: now,
                ingested_at: now,
                text: "Sarah promised the user research by Wednesday; I'll send Marco the pricing quote by Friday".into(),
                redactions: vec![],
                session: None,
                speaker: None,
                content_hash: 0,
                provenance: homn_types::Provenance {
                    source_id: "screen_ocr".into(),
                    upstream_ref: "t".into(),
                },
            })
            .await;

        let commitments = Arc::new(MemoryCommitments::new());
        commitments.push(homn_types::Commitment {
            id: CommitmentId::new(),
            text: "I'll send Marco the pricing quote by Friday".into(),
            owner: Party::Me,
            counterpart: Some(Party::Entity("Marco".into())),
            due: Some(chrono::Utc::now() + chrono::Duration::days(7)),
            status: homn_types::CommitmentStatus::Open,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: now,
        });

        let beliefs = Arc::new(MemoryBeliefs::new());
        beliefs.push(homn_types::Belief {
            id: BeliefId::new(),
            topic: "the brain".into(),
            position: "agidb is good enough after the gate".into(),
            confidence: 1.0,
            valid_from: now,
            superseded_at: None,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: now,
        });

        let forget = Arc::new(MemoryForget::returning(homn_types::DeletionReceipt {
            scope: ForgetScope::Entity("Sarah".into()),
            match_count: 1,
            at: now,
        }));

        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "test.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        let state = McpState {
            engine,
            rules: Arc::new(ArcSwap::from_pointee(rules)),
            audit,
            brain: Some(brain as Arc<dyn Brain>),
            commitments: Some(commitments as Arc<dyn Commitments>),
            beliefs: Some(beliefs as Arc<dyn Beliefs>),
            forget: Some(forget as Arc<dyn Forget>),
        };
        let server = HomnMcpServer::new(state);

        // 1. recall — surfaces the observation with provenance.
        let r = server
            .recall(Parameters(RecallArgs {
                cue: "Sarah pricing".into(),
                as_of: None,
                k: 5,
            }))
            .await
            .unwrap();
        let hits: Vec<RecallHit> =
            serde_json::from_str(&r.content[0].as_text().unwrap().text).unwrap();
        assert!(!hits.is_empty(), "recall surfaces the observation");
        assert!(
            !hits[0].source.is_empty(),
            "recall hit carries provenance (source)"
        );
        assert!(
            !hits[0].observation_id.is_empty(),
            "recall hit carries observation_id"
        );

        // 2. timeline — chronological entries for today.
        let day = now.date_naive().format("%Y-%m-%d").to_string();
        let t = server
            .timeline(Parameters(TimelineArgs {
                subject: "".into(),
                from: format!("{day}T00:00:00Z"),
                to: format!("{day}T23:59:59Z"),
            }))
            .await
            .unwrap();
        let entries: Vec<TimelineEntry> =
            serde_json::from_str(&t.content[0].as_text().unwrap().text).unwrap();
        assert!(!entries.is_empty(), "timeline has today's entry");

        // 3. commitments — the open Marco pricing commitment.
        let c = server
            .commitments(Parameters(CommitmentsArgs {
                status: Some("open".into()),
                due_before: None,
            }))
            .await
            .unwrap();
        let cs: Vec<homn_types::Commitment> =
            serde_json::from_str(&c.content[0].as_text().unwrap().text).unwrap();
        assert!(
            cs.iter().any(|x| x.text.contains("pricing quote")),
            "commitments returns the promise"
        );

        // 4. beliefs — the current brain position.
        let b = server
            .beliefs(Parameters(BeliefsArgs {
                topic: "brain".into(),
                current_only: true,
            }))
            .await
            .unwrap();
        let bs: Vec<homn_types::Belief> =
            serde_json::from_str(&b.content[0].as_text().unwrap().text).unwrap();
        assert!(
            bs.iter()
                .any(|x| x.position.contains("agidb is good enough")),
            "beliefs returns the position"
        );

        // 5. whodis — Sarah's dossier (interactions + open loops).
        let w = server
            .whodis(Parameters(WhodisArgs {
                name: "Sarah".into(),
            }))
            .await
            .unwrap();
        let wv: serde_json::Value =
            serde_json::from_str(&w.content[0].as_text().unwrap().text).unwrap();
        assert!(
            wv["interactions_count"].as_u64().unwrap() >= 1,
            "whodis counts Sarah interactions"
        );

        // 6. today — the day's recap.
        let td = server
            .today(Parameters(TodayArgs {
                date: Some(day.clone()),
            }))
            .await
            .unwrap();
        let tdv: serde_json::Value =
            serde_json::from_str(&td.content[0].as_text().unwrap().text).unwrap();
        assert!(
            tdv["did_count"].as_u64().unwrap() >= 1,
            "today has the day's timeline"
        );

        // 7. forget — the one write tool; returns a receipt proving the scope.
        let f = server
            .forget(Parameters(ForgetArgs {
                entity: Some("Sarah".into()),
                pattern: None,
                from: None,
                to: None,
            }))
            .await
            .unwrap();
        let fv: serde_json::Value =
            serde_json::from_str(&f.content[0].as_text().unwrap().text).unwrap();
        assert!(
            fv["match_count"].as_u64().unwrap() >= 1,
            "forget returns the match count"
        );
        assert!(
            fv["scope"]["kind"] == "entity",
            "forget proves the scope without re-exposing content"
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
