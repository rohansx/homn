//! Decision records: the row written to `audit.db` for every policy evaluation.
//!
//! See [`specs/001-policy-engine/data-model.md`](../../../specs/001-policy-engine/data-model.md)
//! for the schema this type round-trips with.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::session::SessionId;

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// The tool call may proceed.
    Allow,
    /// The tool call is denied.
    Deny,
    /// The decision is deferred to a human surface (TUI / face / ntfy).
    Ask,
}

/// The answer a human gave when they resolved an [`Decision::Ask`].
///
/// `AlwaysAllow` / `AlwaysDeny` are stronger forms — they trigger a learning suggestion to
/// promote a rule, never silently modify policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanAnswer {
    /// One-time allow.
    Allow,
    /// One-time deny.
    Deny,
    /// Allow + offer a rule promotion that would auto-allow this pattern next time.
    AlwaysAllow,
    /// Deny + offer a rule promotion that would auto-deny this pattern next time.
    AlwaysDeny,
}

/// Which UI surface answered an `ask` decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Terminal UI in the calling session's terminal.
    Tui,
    /// The expressive ASCII face (Phase 2; opt-in).
    Face,
    /// ntfy mobile push.
    Ntfy,
    /// MCP introspection — the agent answered itself (rare).
    Mcp,
    /// No surface — the hook returned without asking a human.
    #[serde(rename = "hook-direct")]
    HookDirect,
}

/// Where the decision request originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionSource {
    /// Claude Code `PermissionRequest` hook.
    Hook,
    /// The PTY-tap wrapper (`homn run claude ...`).
    PtyWrapper,
    /// MCP introspection via `query_policy` (dry run; not written to audit).
    Mcp,
}

/// File + line of the rule that fired.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleSourceLocation {
    /// Path relative to the policies directory (e.g. `default.rhai`).
    pub file: PathBuf,
    /// 1-indexed line number where the matching rule begins.
    pub line: u32,
}

/// Optional context returned by ctxgraph (Phase 3+). `None` in Phase 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionContext {
    /// The wiki page slug that informed the decision.
    pub wiki_page: String,
    /// First N chars of the matching section.
    pub excerpt: String,
    /// Cosine similarity (0.0–1.0).
    pub confidence: f32,
}

/// A row in `audit.db` — one decision recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    /// Auto-incremented row id from SQLite.
    pub id: i64,
    /// Unix epoch milliseconds.
    pub ts_millis: i64,
    /// The Claude Code session that triggered this decision.
    pub session_id: SessionId,
    /// Working directory of the calling session.
    pub cwd: PathBuf,
    /// Tool name (e.g. `"Bash"`, `"Read"`, `"mcp__server__tool"`).
    pub tool_name: String,
    /// Tool input as JSON (capped 4 KiB before persisting).
    pub tool_input: serde_json::Value,
    /// The deterministic decision the engine made.
    pub decision: Decision,
    /// If `decision == Ask`, the answer the human gave.
    pub human_answer: Option<HumanAnswer>,
    /// File + line of the rule that fired (None if no rule matched).
    pub rule_source: Option<RuleSourceLocation>,
    /// Snapshot of the rule's text (so the audit row remains readable even if the rule is edited later).
    pub rule_text: Option<String>,
    /// Ctxgraph context (Phase 3+).
    pub ctxgraph_hit: Option<DecisionContext>,
    /// End-to-end latency from request to decision.
    pub latency_ms: u32,
    /// Which surface answered (if any).
    pub surface: Option<Surface>,
    /// Where the decision request came from.
    pub source: DecisionSource,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn decision_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&Decision::Allow).unwrap(),
            "\"allow\""
        );
        assert_eq!(serde_json::to_string(&Decision::Deny).unwrap(), "\"deny\"");
        assert_eq!(serde_json::to_string(&Decision::Ask).unwrap(), "\"ask\"");
    }

    #[test]
    fn decision_deserializes_from_snake_case() {
        let d: Decision = serde_json::from_str("\"deny\"").unwrap();
        assert_eq!(d, Decision::Deny);
    }

    #[test]
    fn human_answer_round_trips_with_underscores() {
        let json = serde_json::to_string(&HumanAnswer::AlwaysAllow).unwrap();
        assert_eq!(json, "\"always_allow\"");
        let parsed: HumanAnswer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, HumanAnswer::AlwaysAllow);
    }

    #[test]
    fn surface_hook_direct_uses_kebab() {
        let json = serde_json::to_string(&Surface::HookDirect).unwrap();
        assert_eq!(json, "\"hook-direct\"");
        let parsed: Surface = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Surface::HookDirect);
    }

    #[test]
    fn decision_source_uses_kebab() {
        let json = serde_json::to_string(&DecisionSource::PtyWrapper).unwrap();
        assert_eq!(json, "\"pty-wrapper\"");
        let parsed: DecisionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DecisionSource::PtyWrapper);
    }

    #[test]
    fn rule_source_location_round_trips() {
        let loc = RuleSourceLocation {
            file: PathBuf::from("policies/default.rhai"),
            line: 14,
        };
        let json = serde_json::to_string(&loc).unwrap();
        let parsed: RuleSourceLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, loc);
        assert!(json.contains("\"line\":14"));
    }

    #[test]
    fn decision_record_round_trips_with_optional_fields() {
        let rec = DecisionRecord {
            id: 42,
            ts_millis: 1_715_587_200_000,
            session_id: SessionId::new("01HXY"),
            cwd: PathBuf::from("/home/rsx/dev/cloakpipe"),
            tool_name: "Bash".to_owned(),
            tool_input: serde_json::json!({"command": "git push origin main"}),
            decision: Decision::Deny,
            human_answer: None,
            rule_source: Some(RuleSourceLocation {
                file: PathBuf::from("default.rhai"),
                line: 10,
            }),
            rule_text: Some(
                "deny if tool == \"Bash\" && cmd.matches(\"git push * main\")".to_owned(),
            ),
            ctxgraph_hit: None,
            latency_ms: 47,
            surface: Some(Surface::HookDirect),
            source: DecisionSource::Hook,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let parsed: DecisionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rec);
    }
}
