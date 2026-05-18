//! Rhai-based policy engine.
//!
//! Parses `.rhai` policy files (lines of `<verb> if <expr>;`) and evaluates them in
//! **deny → ask → allow** order. First matching rule in priority order wins; if no rule
//! matches, the default decision is `Ask`.
//!
//! See [`docs/technical/policy-language.md`](../../../docs/technical/policy-language.md) for the
//! DSL, [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md) for
//! architecture, and [`specs/001-policy-engine/spec.md`](../../../specs/001-policy-engine/spec.md)
//! §"User Story 1" for the acceptance scenarios.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use homn_types::{Decision, RuleSourceLocation};
use rhai::Engine as RhaiEngine;

mod parse;
pub mod watcher;

pub use parse::{ParseError, RuleSet};
pub use watcher::{spawn_reloader, Reloader, RuleSetHandle};

/// The outcome of a single policy evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The decision the engine reached.
    pub decision: Decision,
    /// File + line of the rule that fired, if any.
    pub rule: Option<RuleSourceLocation>,
    /// Snapshot of the rule's text (for retro-readability in the audit log).
    pub rule_text: Option<String>,
}

impl Outcome {
    /// Build an [`Outcome::Ask`] with no firing rule — the "no match → default ask" case.
    pub fn default_ask() -> Self {
        Self {
            decision: Decision::Ask,
            rule: None,
            rule_text: None,
        }
    }
}

/// One rule's contribution to a [`Trace`].
#[derive(Debug, Clone, PartialEq)]
pub struct RuleTrace {
    /// The decision this rule produces when it fires (`deny`, `ask`, or `allow`).
    pub verb: Decision,
    /// File + line of the rule.
    pub location: RuleSourceLocation,
    /// The rule's full source text.
    pub source_text: String,
    /// Whether the rule matched this request.
    pub matched: bool,
    /// Whether this rule is the one that decided the outcome — the first match in
    /// deny → ask → allow priority order. At most one rule in a trace is decisive.
    pub decisive: bool,
}

/// A full evaluation trace: every rule, in evaluation order, plus the final outcome.
///
/// Where [`Engine::eval`] short-circuits on the first match, [`Engine::trace`] evaluates
/// *every* rule so a human can see exactly what did and didn't fire — the data behind
/// `homn rule trace`.
#[derive(Debug, Clone)]
pub struct Trace {
    /// Every rule, evaluated in deny → ask → allow order.
    pub rules: Vec<RuleTrace>,
    /// The decision the engine reached (the decisive rule, or default-ask if none matched).
    pub outcome: Outcome,
}

/// Per-tool-call evaluation context, bound into Rhai's scope.
#[derive(Debug, Clone, Default)]
pub struct EvalRequest {
    /// Tool name (e.g. `"Bash"`).
    pub tool: String,
    /// For `Bash`: the command. Empty otherwise.
    pub cmd: String,
    /// For `Read` / `Edit` / `Write`: the file path. Empty otherwise.
    pub path: String,
    /// For `WebFetch`: the URL. Empty otherwise.
    pub url: String,
    /// Working directory of the calling session.
    pub cwd: String,
    /// `$HOME`.
    pub home: String,
    /// Session ULID from Claude Code.
    pub session_id: String,
}

impl EvalRequest {
    /// Pull `cmd` / `path` / `url` from a generic `tool_input` JSON object.
    ///
    /// Convenience constructor for the daemon: given `tool_name` + `tool_input`, build a request
    /// with the relevant scope variables already populated.
    pub fn from_tool_call(tool_name: &str, tool_input: &serde_json::Value, cwd: &str) -> Self {
        let cmd = tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let path = tool_input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let url = tool_input
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            tool: tool_name.to_owned(),
            cmd,
            path,
            url,
            cwd: cwd.to_owned(),
            home,
            session_id: String::new(),
        }
    }
}

/// The policy engine.
///
/// Wraps a single Rhai [`Engine`](rhai::Engine) with configured sandbox limits and our custom
/// helper functions (`matches`, `regex`).
#[derive(Clone)]
pub struct Engine {
    inner: std::sync::Arc<RhaiEngine>,
}

impl Engine {
    /// Build an engine with default sandbox limits (suitable for production).
    pub fn new() -> Self {
        let mut inner = RhaiEngine::new();
        inner.set_max_operations(100_000);
        inner.set_max_call_levels(32);
        inner.set_max_string_size(8 * 1024);
        inner.set_max_array_size(1024);
        inner.set_max_modules(8);
        inner.set_max_expr_depths(64, 64);
        register_helpers(&mut inner);
        Self {
            inner: std::sync::Arc::new(inner),
        }
    }

    /// Override the per-rule operation budget. Useful for tests; default is 100_000.
    pub fn with_max_operations(mut self, ops: u64) -> Self {
        let inner = std::sync::Arc::get_mut(&mut self.inner)
            .expect("with_max_operations requires unique ownership");
        inner.set_max_operations(ops);
        self
    }

    /// Borrow the underlying Rhai engine (used by `parse::compile_rule`).
    pub(crate) fn rhai(&self) -> &RhaiEngine {
        &self.inner
    }

    /// Evaluate a [`RuleSet`] against a [`EvalRequest`].
    ///
    /// Iterates rules in **deny → ask → allow** order; first matching rule wins. If no rule
    /// matches, returns [`Outcome::default_ask`]. Rules that fail to evaluate (e.g. operation
    /// budget exhausted) are logged and treated as non-matches.
    pub fn eval(&self, rules: &RuleSet, req: &EvalRequest) -> Outcome {
        for rule in rules.deny_rules() {
            if self.fires(rule, req) {
                return self.outcome(Decision::Deny, rule);
            }
        }
        for rule in rules.ask_rules() {
            if self.fires(rule, req) {
                return self.outcome(Decision::Ask, rule);
            }
        }
        for rule in rules.allow_rules() {
            if self.fires(rule, req) {
                return self.outcome(Decision::Allow, rule);
            }
        }
        Outcome::default_ask()
    }

    /// Evaluate a [`RuleSet`] like [`eval`](Self::eval), but without short-circuiting.
    ///
    /// Returns a [`Trace`]: every rule, in deny → ask → allow order, each tagged with whether
    /// it matched and whether it was the decisive (first-match) rule. The reachable outcome is
    /// identical to [`eval`](Self::eval) — `trace` exists so `homn rule trace` can show *why*.
    pub fn trace(&self, rules: &RuleSet, req: &EvalRequest) -> Trace {
        let ordered = rules
            .deny_rules()
            .map(|r| (Decision::Deny, r))
            .chain(rules.ask_rules().map(|r| (Decision::Ask, r)))
            .chain(rules.allow_rules().map(|r| (Decision::Allow, r)));

        let mut traced = Vec::new();
        let mut outcome: Option<Outcome> = None;

        for (verb, rule) in ordered {
            let matched = self.fires(rule, req);
            // The first matching rule, in priority order, decides — and only it.
            let decisive = matched && outcome.is_none();
            if decisive {
                outcome = Some(self.outcome(verb, rule));
            }
            traced.push(RuleTrace {
                verb,
                location: RuleSourceLocation {
                    file: PathBuf::from(rule.file_name()),
                    line: rule.line(),
                },
                source_text: rule.source_text().to_owned(),
                matched,
                decisive,
            });
        }

        Trace {
            rules: traced,
            outcome: outcome.unwrap_or_else(Outcome::default_ask),
        }
    }

    fn fires(&self, rule: &parse::CompiledRule, req: &EvalRequest) -> bool {
        let mut scope = rhai::Scope::new();
        scope.push_constant("tool", req.tool.clone());
        scope.push_constant("cmd", req.cmd.clone());
        scope.push_constant("path", req.path.clone());
        scope.push_constant("url", req.url.clone());
        scope.push_constant("cwd", req.cwd.clone());
        scope.push_constant("home", req.home.clone());
        scope.push_constant("session_id", req.session_id.clone());

        match self
            .inner
            .eval_ast_with_scope::<bool>(&mut scope, rule.ast())
        {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(
                    file = %rule.file_name(),
                    line = rule.line(),
                    error = %err,
                    "policy rule evaluation failed; treating as non-match"
                );
                false
            }
        }
    }

    fn outcome(&self, decision: Decision, rule: &parse::CompiledRule) -> Outcome {
        Outcome {
            decision,
            rule: Some(RuleSourceLocation {
                file: PathBuf::from(rule.file_name()),
                line: rule.line(),
            }),
            rule_text: Some(rule.source_text().to_owned()),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Register our custom helpers on a Rhai engine: `matches` (glob) and `regex` (RE2-flavoured).
fn register_helpers(engine: &mut RhaiEngine) {
    engine.register_fn("matches", |s: &str, pattern: &str| glob_match(s, pattern));
    engine.register_fn("matches", |s: String, pattern: &str| {
        glob_match(&s, pattern)
    });
    engine.register_fn("regex", |s: &str, pattern: &str| -> bool {
        regex::Regex::new(pattern)
            .map(|r| r.is_match(s))
            .unwrap_or(false)
    });
    engine.register_fn("regex", |s: String, pattern: &str| -> bool {
        regex::Regex::new(pattern)
            .map(|r| r.is_match(&s))
            .unwrap_or(false)
    });
}

/// Simple shell-style glob: `*` matches any chars, `?` matches one char, everything else literal.
fn glob_match(text: &str, pattern: &str) -> bool {
    let mut regex_src = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex_src.push_str(".*"),
            '?' => regex_src.push('.'),
            c if "\\.+()[]{}|^$".contains(c) => {
                regex_src.push('\\');
                regex_src.push(c);
            }
            c => regex_src.push(c),
        }
    }
    regex_src.push('$');
    regex::Regex::new(&regex_src)
        .map(|r| r.is_match(text))
        .unwrap_or(false)
}

/// Convenience: load a [`RuleSet`] from disk using a default [`Engine`].
pub fn load_ruleset(path: impl AsRef<Path>) -> Result<RuleSet, ParseError> {
    let engine = Engine::new();
    RuleSet::load(&engine, path.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(tool: &str, cmd: &str) -> EvalRequest {
        EvalRequest {
            tool: tool.into(),
            cmd: cmd.into(),
            home: "/home/rsx".into(),
            cwd: "/home/rsx/dev/x".into(),
            ..Default::default()
        }
    }

    fn ruleset(engine: &Engine, src: &str) -> RuleSet {
        RuleSet::parse(engine, src, "test.rhai").expect("ruleset parses")
    }

    #[test]
    fn glob_match_handles_star_and_question() {
        assert!(glob_match("git push origin main", "git push * main"));
        assert!(glob_match("npm run build", "npm run *"));
        assert!(glob_match("cargo build", "cargo *"));
        assert!(!glob_match("cargo build", "npm *"));
        assert!(glob_match("a", "?"));
        assert!(!glob_match("ab", "?"));
    }

    #[test]
    fn empty_ruleset_yields_default_ask() {
        let eng = Engine::new();
        let rs = ruleset(&eng, "");
        let out = eng.eval(&rs, &req("Bash", "ls"));
        assert_eq!(out.decision, Decision::Ask);
        assert!(out.rule.is_none());
    }

    #[test]
    fn allow_rule_fires_on_match() {
        let eng = Engine::new();
        let rs = ruleset(&eng, r#"allow if tool == "Read";"#);
        let out = eng.eval(&rs, &req("Read", ""));
        assert_eq!(out.decision, Decision::Allow);
        assert!(out.rule.is_some());
    }

    #[test]
    fn deny_rule_beats_allow_rule() {
        // Constitution / spec: deny → ask → allow. A matching deny must win over a matching allow.
        let eng = Engine::new();
        let src = r#"
            allow if tool == "Bash";
            deny if tool == "Bash" && cmd.contains("rm -rf");
        "#;
        let rs = ruleset(&eng, src);
        let out = eng.eval(&rs, &req("Bash", "rm -rf ~/scratch"));
        assert_eq!(out.decision, Decision::Deny);
        let loc = out.rule.unwrap();
        assert_eq!(loc.file, PathBuf::from("test.rhai"));
        // The rule at line 3 in the source above is the deny rule.
        assert_eq!(loc.line, 3);
    }

    #[test]
    fn allow_rule_fires_when_no_deny_matches() {
        let eng = Engine::new();
        let src = r#"
            deny if tool == "Bash" && cmd.contains("rm -rf");
            allow if tool == "Read" && path.starts_with(home);
        "#;
        let rs = ruleset(&eng, src);
        let mut r = req("Read", "");
        r.path = "/home/rsx/foo.txt".into();
        let out = eng.eval(&rs, &r);
        assert_eq!(out.decision, Decision::Allow);
    }

    #[test]
    fn no_match_falls_through_to_default_ask() {
        // Spec §"User Story 1 / Acceptance Scenario 3": unmatched call → ask.
        let eng = Engine::new();
        let src = r#"
            allow if tool == "Read";
            deny if tool == "Bash" && cmd.contains("rm -rf");
        "#;
        let rs = ruleset(&eng, src);
        let out = eng.eval(&rs, &req("WebFetch", ""));
        assert_eq!(out.decision, Decision::Ask);
        assert!(out.rule.is_none());
    }

    #[test]
    fn matches_helper_works_inside_rules() {
        let eng = Engine::new();
        let rs = ruleset(
            &eng,
            r#"allow if tool == "Bash" && cmd.matches("npm run *");"#,
        );
        let out = eng.eval(&rs, &req("Bash", "npm run build"));
        assert_eq!(out.decision, Decision::Allow);
    }

    #[test]
    fn regex_helper_works_inside_rules() {
        let eng = Engine::new();
        let rs = ruleset(
            &eng,
            r#"allow if tool == "Bash" && cmd.regex("^git (status|log|diff)( |$)");"#,
        );
        let out = eng.eval(&rs, &req("Bash", "git status"));
        assert_eq!(out.decision, Decision::Allow);
    }

    #[test]
    fn three_acceptance_scenarios_from_us1_pass() {
        // Mirrors Spec §"User Story 1 / Acceptance Scenarios" 1, 2, and 3.
        let eng = Engine::new();
        let src = r#"
            allow if tool == "Read" && path.starts_with(home);
            deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
        "#;
        let rs = ruleset(&eng, src);

        // (1) Bash rm -rf outside /tmp → deny via the rule.
        let mut r1 = req("Bash", "rm -rf ~/scratch");
        r1.cwd = "/home/rsx/dev/cloakpipe".into();
        assert_eq!(eng.eval(&rs, &r1).decision, Decision::Deny);

        // (2) Read inside home → allow silently.
        let mut r2 = req("Read", "");
        r2.path = "/home/rsx/foo.txt".into();
        let out2 = eng.eval(&rs, &r2);
        assert_eq!(out2.decision, Decision::Allow);
        assert!(out2.rule.is_some());

        // (3) Some other tool with no rule → fall through to ask.
        let r3 = req("WebFetch", "");
        let out3 = eng.eval(&rs, &r3);
        assert_eq!(out3.decision, Decision::Ask);
        assert!(out3.rule.is_none());
    }

    #[test]
    fn all_shipped_sample_policies_parse() {
        // Guard: every file in policies/ must parse cleanly. `include_str!` also makes this a
        // compile-time check that the files still exist at the expected paths.
        let eng = Engine::new();
        for (name, src) in [
            (
                "default.rhai",
                include_str!("../../../policies/default.rhai"),
            ),
            ("strict.rhai", include_str!("../../../policies/strict.rhai")),
            (
                "relaxed.rhai",
                include_str!("../../../policies/relaxed.rhai"),
            ),
            (
                "project-example.rhai",
                include_str!("../../../policies/project-example.rhai"),
            ),
        ] {
            RuleSet::parse(&eng, src, name)
                .unwrap_or_else(|e| panic!("shipped policy `{name}` must parse: {e}"));
        }
    }

    #[test]
    fn default_policy_allows_the_normal_dev_loop() {
        // Regression: an explicit `ask if true` catch-all shadowed every allow rule, because
        // asks are evaluated before allows (deny -> ask -> allow). The shipped default.rhai
        // must rely on the engine's *implicit* fallthrough-to-ask instead.
        let eng = Engine::new();
        let src = include_str!("../../../policies/default.rhai");
        let rs = RuleSet::parse(&eng, src, "default.rhai").expect("default.rhai parses");

        let build = req("Bash", "cargo build --release");
        assert_eq!(
            eng.eval(&rs, &build).decision,
            Decision::Allow,
            "`cargo build` is in the allow list — it must not be shadowed into ask"
        );

        let danger = req("Bash", "rm -rf /etc");
        assert_eq!(
            eng.eval(&rs, &danger).decision,
            Decision::Deny,
            "`rm -rf` outside /tmp is still denied"
        );

        // A genuinely unmatched call still falls through to ask — via the engine default,
        // not a catch-all rule.
        let unknown = req("Bash", "frobnicate the widget");
        let out = eng.eval(&rs, &unknown);
        assert_eq!(out.decision, Decision::Ask);
        assert!(
            out.rule.is_none(),
            "unmatched ask comes from the implicit default"
        );
    }

    #[test]
    fn trace_reports_every_rule_and_marks_the_decisive_one() {
        // T084: `homn rule trace` needs to see *all* rules, not just the winner.
        let eng = Engine::new();
        let src = r#"
            deny if tool == "Bash" && cmd.contains("rm -rf");
            allow if tool == "Bash";
        "#;
        let rs = ruleset(&eng, src);
        let trace = eng.trace(&rs, &req("Bash", "ls -la"));

        assert_eq!(trace.rules.len(), 2, "both rules appear in the trace");

        let deny = &trace.rules[0];
        assert_eq!(deny.verb, Decision::Deny);
        assert!(!deny.matched, "the deny rule does not match `ls -la`");
        assert!(!deny.decisive);

        let allow = &trace.rules[1];
        assert_eq!(allow.verb, Decision::Allow);
        assert!(allow.matched, "the allow rule matches a Bash call");
        assert!(allow.decisive, "the allow rule is the first (only) match");

        assert_eq!(trace.outcome.decision, Decision::Allow);
    }

    #[test]
    fn trace_first_match_in_priority_order_is_decisive() {
        // A matching deny wins over a matching allow even when the allow is listed first.
        let eng = Engine::new();
        let src = r#"
            allow if tool == "Bash";
            deny if tool == "Bash" && cmd.contains("rm -rf");
        "#;
        let rs = ruleset(&eng, src);
        let trace = eng.trace(&rs, &req("Bash", "rm -rf /x"));

        assert_eq!(trace.outcome.decision, Decision::Deny);

        let decisive: Vec<_> = trace.rules.iter().filter(|r| r.decisive).collect();
        assert_eq!(decisive.len(), 1, "exactly one rule is decisive");
        assert_eq!(decisive[0].verb, Decision::Deny);

        // The allow rule still matched — it just lost on priority.
        let allow = trace
            .rules
            .iter()
            .find(|r| r.verb == Decision::Allow)
            .expect("allow rule present");
        assert!(allow.matched);
        assert!(!allow.decisive);
    }

    #[test]
    fn trace_with_no_match_falls_through_to_ask() {
        let eng = Engine::new();
        let rs = ruleset(&eng, r#"allow if tool == "Read";"#);
        let trace = eng.trace(&rs, &req("WebFetch", ""));

        assert_eq!(trace.outcome.decision, Decision::Ask);
        assert!(trace.outcome.rule.is_none());
        assert!(
            trace.rules.iter().all(|r| !r.decisive),
            "no rule is decisive when nothing matched"
        );
    }

    // Budget-enforcement integration test deferred to R-004 criterion bench (see
    // specs/001-policy-engine/research.md). compile_expression deliberately forbids statements
    // (while / for / blocks), so a pathological *parse-time* rule is unrepresentable in v0 —
    // which is the whole point. Runtime starvation of the operations counter is a separate
    // failure mode that's better characterized by benchmarks than a hand-written test case.
}
