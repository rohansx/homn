//! `BusEvent` — broadcast events on the daemon's event bus.
//!
//! Subscribers (TUI, face, ntfy, learning subsystem, audit writer, MCP notifications) read these.
//! See [`docs/technical/ipc-protocol.md`](../../../docs/technical/ipc-protocol.md) §"Event broadcast".

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    decision::{Decision, HumanAnswer, RuleSourceLocation, Surface},
    session::SessionId,
};

/// Why a decision counts as "high stakes" (warrants the alert state on the face).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighStakesKind {
    /// Push to a protected branch (main / master / release/*).
    GitPushProtected,
    /// Any tool call that touches credentials.
    Credentials,
    /// Network call to a production host.
    ProductionNetwork,
    /// Deploy / release-shaped command.
    Deploy,
}

/// Events broadcast on the daemon's event bus.
///
/// Tagged with `kind` so subscribers can filter cheaply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum BusEvent {
    /// A deterministic decision (allow or deny) was made without asking a human.
    DecisionMade {
        /// Audit row id.
        decision_id: i64,
        /// Tool name.
        tool: String,
        /// The decision outcome.
        decision: Decision,
        /// The rule that fired, if any.
        rule: Option<RuleSourceLocation>,
    },
    /// An `Ask` decision has opened on at least one surface — the daemon is waiting for a human.
    AskOpened {
        /// Audit row id.
        decision_id: i64,
        /// Tool name.
        tool: String,
        /// Preview of the tool input (capped 256 chars).
        tool_input_preview: String,
        /// Calling session.
        session_id: SessionId,
        /// Working directory.
        cwd: PathBuf,
    },
    /// An `Ask` decision has been answered by a human (or timed out — `answer = None`).
    AskClosed {
        /// Audit row id.
        decision_id: i64,
        /// The human's answer, or `None` if the timeout elapsed.
        answer: Option<HumanAnswer>,
        /// End-to-end latency from `AskOpened` to here.
        latency_ms: u32,
        /// Which surface answered.
        surface: Option<Surface>,
    },
    /// The learning subsystem has a new rule-promotion suggestion ready.
    LearningSuggestion {
        /// Learning suggestion id (in `learning.db`).
        id: i64,
        /// Human-readable pattern representation.
        pattern_repr: String,
        /// The Rhai rule that would be appended on acceptance.
        proposed_rule: String,
        /// How many consecutive matching answers produced this suggestion.
        observation_count: u32,
    },
    /// Claude Code session started (`SessionStart` hook).
    SessionStarted {
        /// Stable session id.
        session_id: SessionId,
        /// Working directory at session start.
        cwd: PathBuf,
    },
    /// Claude Code session ended (`Stop` hook).
    SessionEnded {
        /// Stable session id.
        session_id: SessionId,
    },
    /// A high-stakes decision is pending — the face should flip to alert state.
    HighStakesPending {
        /// Audit row id.
        decision_id: i64,
        /// Why this is considered high-stakes. (Field is `category` rather than `kind` to avoid
        /// conflict with the enum's internal `#[serde(tag = "kind")]`.)
        category: HighStakesKind,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::Decision;
    use std::path::PathBuf;

    #[test]
    fn decision_made_is_tagged_with_kind() {
        let ev = BusEvent::DecisionMade {
            decision_id: 1,
            tool: "Bash".into(),
            decision: Decision::Allow,
            rule: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "DecisionMade");
        assert_eq!(json["tool"], "Bash");
        assert_eq!(json["decision"], "allow");
    }

    #[test]
    fn ask_opened_round_trips() {
        let ev = BusEvent::AskOpened {
            decision_id: 7,
            tool: "WebFetch".into(),
            tool_input_preview: "https://internal.example.com/...".into(),
            session_id: SessionId::new("01HXY"),
            cwd: PathBuf::from("/home/rsx/dev"),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: BusEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ev);
    }

    #[test]
    fn ask_closed_with_none_answer_serializes() {
        let ev = BusEvent::AskClosed {
            decision_id: 3,
            answer: None,
            latency_ms: 30_000,
            surface: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "AskClosed");
        assert!(json["answer"].is_null());
    }

    #[test]
    fn high_stakes_pending_round_trips_and_uses_kind_tag() {
        let ev = BusEvent::HighStakesPending {
            decision_id: 12,
            category: HighStakesKind::GitPushProtected,
        };
        let json = serde_json::to_value(&ev).unwrap();
        // The outer `kind` field is the BusEvent variant tag.
        assert_eq!(json["kind"], "HighStakesPending");
        // The inner `category` field carries the HighStakesKind, snake_case-rendered.
        assert_eq!(json["category"], "git_push_protected");
        // Round-trip:
        let s = serde_json::to_string(&ev).unwrap();
        let parsed: BusEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, ev);
    }

    #[test]
    fn learning_suggestion_round_trips() {
        let ev = BusEvent::LearningSuggestion {
            id: 99,
            pattern_repr: "Bash: git push origin feat/*".into(),
            proposed_rule: "allow if tool == \"Bash\" && cmd.matches(\"git push origin feat/*\")".into(),
            observation_count: 5,
        };
        let s = serde_json::to_string(&ev).unwrap();
        let parsed: BusEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, ev);
    }
}
