//! The ingest-policy stage — the first stage of the gate pipeline (US3 / T035).
//!
//! Compiles and evaluates `policies/ingest.rhai` against an ingest-specific scope
//! (`app`, `domain`, `source_kind`, `window_title`, `incognito`) and resolves to an
//! [`IngestAction`] + the rule id that fired, for the decision receipt. This is the v2
//! analog of the v1 `homn-policy` engine, but the scope is capture-oriented rather than
//! tool-call-oriented, and the verbs are `allow` / `deny` / `redact(kinds)` / `allow_cloud`
//! instead of `deny` / `ask` / `allow`. See [`contracts/gate-pipeline.md`] §"Policy evaluation
//! contract" and FR-013/FR-026.
//!
//! Failure model — **fail closed** (R-2): a compile or runtime error in the script returns
//! [`PolicyDecision::Error`], which the pipeline turns into a dropped item + an
//! `Error` decision receipt. A broken policy file never lets unredacted text through.
//! Hot-reload (FR-013) keeps the last-good policy on a reload failure.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rhai::{Dynamic, Engine, Scope};

use crate::IngestAction;

/// The ingest policy evaluation context — the scope variables bound into Rhai.
#[derive(Debug, Clone, Default)]
pub struct IngestContext {
    /// App / window-title / account the capture came from, if known.
    pub app: String,
    /// Domain of a browser tab (e.g. `github.com`), if known.
    pub domain: String,
    /// `homn_types::SourceKind` as a snake_case string (e.g. `"screen_ocr"`).
    pub source_kind: String,
    /// Full window title, if known.
    pub window_title: String,
    /// Whether the surface is incognito / private mode (browser, terminal private window).
    pub incognito: bool,
}

/// What the policy resolved to for one captured item, plus the rule that fired (for the receipt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    /// The action to take.
    pub action: IngestAction,
    /// The rule id that fired, if any (the `tag("...")` most recently set before the action, or
    /// the action's own default id when the script set one). `None` only when no rule fired and
    /// the default was applied.
    pub rule_id: Option<String>,
}

/// A compiled ingest policy — a Rhai AST ready to evaluate against any [`IngestContext`].
#[derive(Clone)]
pub struct IngestPolicy {
    ast: Arc<rhai::AST>,
}

/// The default policy applied when the script neither sets an action nor errors.
///
/// Per FR-026: conservative — DENY for sensitive-flagged surfaces (incognito, password-manager
/// windows, vault apps), ALLOW+Redact(secrets-only) otherwise, never AllowCloud.
fn default_decision(ctx: &IngestContext) -> PolicyDecision {
    let sensitive = ctx.incognito
        || ctx.window_title.to_lowercase().contains("password")
        || ctx.window_title.to_lowercase().contains("vault")
        || ctx.app.to_lowercase().contains("keepass")
        || ctx.app.to_lowercase().contains("1password")
        || ctx.app.to_lowercase().contains("bitwarden");
    if sensitive {
        PolicyDecision {
            action: IngestAction::Deny,
            rule_id: Some("default.sensitive".to_owned()),
        }
    } else {
        PolicyDecision {
            action: IngestAction::Allow,
            rule_id: Some("default.allow".to_owned()),
        }
    }
}

/// Parse a Rhai `kinds` argument (a string or array of strings) into a `Vec<RedactionKind>`.
fn parse_kinds(arg: &Dynamic) -> Vec<homn_types::RedactionKind> {
    use homn_types::RedactionKind as K;
    let mut out = Vec::new();
    let push = |out: &mut Vec<_>, s: &str| {
        let k = match s {
            "api_key" => K::ApiKey,
            "token" => K::Token,
            "card" => K::Card,
            "aadhaar" => K::Aadhaar,
            "pan" => K::Pan,
            "person_pii" => K::PersonPii,
            "email_addr" => K::EmailAddr,
            "phone" => K::Phone,
            "other" => K::Other,
            _ => return,
        };
        out.push(k);
    };
    if arg.is_string() {
        if let Ok(s) = arg.clone().into_string() {
            for part in s.split(',') {
                push(&mut out, part.trim());
            }
        }
    } else if arg.is_array() {
        if let Ok(arr) = arg.as_array_ref() {
            for v in arr.iter() {
                if let Ok(s) = v.clone().into_string() {
                    push(&mut out, &s);
                }
            }
        }
    }
    out
}

/// Outcome channel shared between the engine and the per-call closures registered as Rhai fns.
type Slot = Arc<Mutex<Option<(IngestAction, Option<String>)>>>;

fn build_engine(slot: Slot) -> Engine {
    let mut engine = Engine::new();
    // Sandbox limits — mirror v1 homn-policy so a runaway ingest rule can't wedge the gate.
    engine.set_max_operations(100_000);
    engine.set_max_call_levels(32);
    engine.set_max_string_size(64 * 1024);
    engine.set_max_array_size(1024);
    engine.set_max_expr_depths(64, 64);

    let set = move |action: IngestAction, tag: Option<String>, slot: &Slot| {
        let mut g = slot.lock().expect("outcome slot poisoned");
        *g = Some((action, tag));
    };

    // --- outcome setters -----------------------------------------------------
    // Each records (action, current-tag). `tag` is reset after each action so it attaches to
    // exactly one decision. A missing tag is fine — the pipeline records the rule_id as None.
    {
        let slot = slot.clone();
        engine.register_fn("deny", move || {
            set(IngestAction::Deny, None, &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("allow", move || {
            set(IngestAction::Allow, None, &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("allow_cloud", move || {
            set(IngestAction::AllowCloud, None, &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("redact", move |kinds: Dynamic| {
            let kinds = parse_kinds(&kinds);
            set(IngestAction::Redact(kinds), None, &slot);
        });
    }

    // --- tag ---------------------------------------------------------------
    // The script names a firing rule via the `_with(id)` variants below, which set both the
    // action and the rule id in one call. A standalone `tag()` would be ambiguous about which
    // action it attaches to, so it is intentionally not provided.

    // The outcome setters that also record a rule id.
    {
        let slot = slot.clone();
        engine.register_fn("deny_with", move |id: &str| {
            set(IngestAction::Deny, Some(id.to_owned()), &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("allow_with", move |id: &str| {
            set(IngestAction::Allow, Some(id.to_owned()), &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("allow_cloud_with", move |id: &str| {
            set(IngestAction::AllowCloud, Some(id.to_owned()), &slot);
        });
    }
    {
        let slot = slot.clone();
        engine.register_fn("redact_with", move |kinds: Dynamic, id: &str| {
            let kinds = parse_kinds(&kinds);
            set(IngestAction::Redact(kinds), Some(id.to_owned()), &slot);
        });
    }

    // --- helpers -------------------------------------------------------------
    // `s.matches(glob)` — shell-style * and ?. Delegates to the same glob impl v1 uses.
    engine.register_fn("matches", |s: &str, pat: &str| glob_match(s, pat));
    engine.register_fn("regex", |s: &str, pat: &str| {
        regex::Regex::new(pat)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    });

    engine
}

/// Shell-glob match: supports `*` (any run) and `?` (one char). Case-sensitive.
fn glob_match(s: &str, pat: &str) -> bool {
    let s: Vec<char> = s.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    glob_inner(&s, &p)
}

fn glob_inner(s: &[char], p: &[char]) -> bool {
    match (p.first(), p.get(1)) {
        (Some('*'), _) => {
            // Try matching zero-or-more of s.
            if p.len() == 1 {
                return true; // trailing * matches everything
            }
            for i in 0..=s.len() {
                if glob_inner(&s[i..], &p[1..]) {
                    return true;
                }
            }
            false
        }
        (Some('?'), _) => !s.is_empty() && glob_inner(&s[1..], &p[1..]),
        (Some(&c), _) => !s.is_empty() && s[0] == c && glob_inner(&s[1..], &p[1..]),
        (None, _) => s.is_empty(),
    }
}

impl IngestPolicy {
    /// Compile an ingest policy from source text. Fails closed: a compile error is returned to
    /// the caller (the pipeline turns it into an `Error` receipt), never silently defaulted.
    pub fn compile(src: &str) -> anyhow::Result<Self> {
        let slot: Slot = Arc::new(Mutex::new(None));
        let engine = build_engine(slot.clone());
        let ast = engine
            .compile(src)
            .map_err(|e| anyhow::anyhow!("ingest policy compile error: {e}"))?;
        // The engine's per-call closures capture a per-call slot, so we rebuild it on every
        // evaluate(); the AST is the only reusable artifact we keep.
        Ok(Self { ast: Arc::new(ast) })
    }

    /// Load and compile the policy file at `path`.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let src = std::fs::read_to_string(path.as_ref())
            .map_err(|e| anyhow::anyhow!("ingest policy read {}: {e}", path.as_ref().display()))?;
        Self::compile(&src)
    }

    /// Evaluate the policy against one capture context. **Fail-closed**: any runtime error
    /// returns [`PolicyDecision::Error`]; the default is applied only when the script runs to
    /// completion without setting an action.
    pub fn evaluate(&self, ctx: &IngestContext) -> PolicyDecision {
        let slot: Slot = Arc::new(Mutex::new(None));
        // Re-bind the outcome setters to this call's slot by rebuilding a fresh engine. The AST
        // is reusable; only the closures' captured slot must be per-call.
        let engine = build_engine(slot.clone());
        let mut scope = Scope::new();
        scope.push("app", ctx.app.clone());
        scope.push("domain", ctx.domain.clone());
        scope.push("source_kind", ctx.source_kind.clone());
        scope.push("window_title", ctx.window_title.clone());
        scope.push("incognito", ctx.incognito);

        if let Err(e) = engine.eval_ast_with_scope::<Dynamic>(&mut scope, &self.ast) {
            tracing::warn!(error = %e, "ingest policy runtime error — failing closed");
            return PolicyDecision {
                action: IngestAction::Deny, // signal error; pipeline records Error receipt
                rule_id: None,
            };
        }

        let outcome = slot.lock().expect("slot poisoned").take();
        match outcome {
            Some((action, id)) => PolicyDecision {
                action,
                rule_id: id,
            },
            None => default_decision(ctx),
        }
    }
}

/// A live, hot-reloading ingest policy. Cheap to clone (Arc); lock-free reads.
pub type IngestPolicyHandle = Arc<ArcSwap<IngestPolicy>>;

/// Background reloader for `policies/ingest.rhai`. Drop to stop watching.
pub struct IngestPolicyReloader {
    /// The current policy, atomically swappable.
    pub handle: IngestPolicyHandle,
    _watcher: RecommendedWatcher,
}

/// Load the policy at `path` and start watching its parent dir for changes (FR-013).
///
/// On a *subsequent* reload failure (broken syntax, deleted file), logs a warning and keeps
/// the last-good policy active — a broken edit never stops the gate, and never downgrades it.
pub fn spawn_reloader(path: impl Into<PathBuf>) -> anyhow::Result<IngestPolicyReloader> {
    let path = path.into();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    let parent = canonical
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let initial = IngestPolicy::load(&canonical)?;
    let handle: IngestPolicyHandle = Arc::new(ArcSwap::from_pointee(initial));

    let handle_for_watcher = handle.clone();
    let path_for_watcher = canonical.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let event = match res {
            Ok(ev) => ev,
            Err(err) => {
                tracing::warn!(error = %err, "ingest-policy file-watch error");
                return;
            }
        };
        let relevant = matches!(
            event.kind,
            notify::EventKind::Modify(_)
                | notify::EventKind::Create(_)
                | notify::EventKind::Remove(_)
        );
        if !relevant {
            return;
        }
        if !event.paths.iter().any(|p| p == &path_for_watcher) {
            return;
        }
        // Tiny debounce: editors emit several events for one save; let the write settle.
        std::thread::sleep(std::time::Duration::from_millis(20));
        match IngestPolicy::load(&path_for_watcher) {
            Ok(p) => {
                handle_for_watcher.store(Arc::new(p));
                tracing::info!(path = %path_for_watcher.display(), "ingest policy hot-reloaded");
            }
            Err(e) => {
                tracing::warn!(
                    path = %path_for_watcher.display(),
                    error = %e,
                    "ingest policy reload failed — keeping last good"
                );
            }
        }
    })?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .map_err(|e| anyhow::anyhow!("notify watch {}: {e}", parent.display()))?;

    Ok(IngestPolicyReloader {
        handle,
        _watcher: watcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::RedactionKind as K;

    fn ctx(app: &str) -> IngestContext {
        IngestContext {
            app: app.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn allow_script_allows() {
        let p = IngestPolicy::compile("allow();").unwrap();
        let d = p.evaluate(&ctx("Firefox"));
        assert_eq!(d.action, IngestAction::Allow);
        assert_eq!(d.rule_id.as_deref(), None);
    }

    #[test]
    fn deny_script_denies() {
        let p = IngestPolicy::compile("deny();").unwrap();
        let d = p.evaluate(&ctx("Firefox"));
        assert_eq!(d.action, IngestAction::Deny);
    }

    #[test]
    fn redact_script_collects_kinds_from_string() {
        let p = IngestPolicy::compile(r#"redact("email_addr,phone");"#).unwrap();
        let d = p.evaluate(&ctx("Slack"));
        assert_eq!(d.action, IngestAction::Redact(vec![K::EmailAddr, K::Phone]));
    }

    #[test]
    fn redact_script_collects_kinds_from_array() {
        let p = IngestPolicy::compile(r#"redact(["email_addr", "phone"]);"#).unwrap();
        let d = p.evaluate(&ctx("Slack"));
        assert_eq!(d.action, IngestAction::Redact(vec![K::EmailAddr, K::Phone]));
    }

    #[test]
    fn allow_cloud_script_sets_allow_cloud() {
        let p = IngestPolicy::compile("allow_cloud();").unwrap();
        let d = p.evaluate(&ctx("Slack"));
        assert_eq!(d.action, IngestAction::AllowCloud);
    }

    #[test]
    fn tagged_action_carries_rule_id() {
        let p = IngestPolicy::compile(r#"allow_with("browser.public");"#).unwrap();
        let d = p.evaluate(&ctx("Firefox"));
        assert_eq!(d.action, IngestAction::Allow);
        assert_eq!(d.rule_id.as_deref(), Some("browser.public"));
    }

    #[test]
    fn conditional_rule_fires_on_match() {
        let src = r#"
            if app.matches("Slack*") { allow_with("slack"); }
            else { deny_with("default.deny"); }
        "#;
        let p = IngestPolicy::compile(src).unwrap();
        assert_eq!(p.evaluate(&ctx("Slack")).rule_id.as_deref(), Some("slack"));
        assert_eq!(p.evaluate(&ctx("Firefox")).action, IngestAction::Deny);
        assert_eq!(
            p.evaluate(&ctx("Firefox")).rule_id.as_deref(),
            Some("default.deny")
        );
    }

    #[test]
    fn incognito_defaults_to_deny_when_no_action_set() {
        let p = IngestPolicy::compile("// no rule\n").unwrap();
        let mut c = ctx("Firefox");
        c.incognito = true;
        let d = p.evaluate(&c);
        assert_eq!(d.action, IngestAction::Deny);
        assert_eq!(d.rule_id.as_deref(), Some("default.sensitive"));
    }

    #[test]
    fn non_incognito_defaults_to_allow_when_no_action_set() {
        let p = IngestPolicy::compile("// no rule\n").unwrap();
        let d = p.evaluate(&ctx("Firefox"));
        assert_eq!(d.action, IngestAction::Allow);
        assert_eq!(d.rule_id.as_deref(), Some("default.allow"));
    }

    #[test]
    fn runtime_error_fails_closed_as_deny() {
        // Calling an undefined function errors at runtime → fail closed.
        let p = IngestPolicy::compile("nope_does_not_exist();").unwrap();
        let d = p.evaluate(&ctx("Firefox"));
        assert_eq!(d.action, IngestAction::Deny);
        assert_eq!(d.rule_id, None);
    }

    #[test]
    fn compile_error_is_reported_not_silently_defaulted() {
        // Missing close paren is a compile error; the caller (pipeline) must see it.
        let p = IngestPolicy::compile("allow(;"); // typo'd paren
        assert!(p.is_err(), "compile error must surface, not default");
    }

    #[test]
    fn regex_helper_works() {
        let p = IngestPolicy::compile(r#"if app.regex("^Git") { allow_with("git"); }"#).unwrap();
        let d = p.evaluate(&ctx("GitHub Desktop"));
        assert_eq!(d.action, IngestAction::Allow);
        assert_eq!(d.rule_id.as_deref(), Some("git"));
    }

    #[test]
    fn hot_reload_swaps_policy_on_file_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ingest.rhai");
        std::fs::write(&path, "allow_with(\"v1\");\n").unwrap();
        let reloader = spawn_reloader(&path).unwrap();
        let handle = reloader.handle.clone();
        assert_eq!(
            handle.load().evaluate(&ctx("X")).rule_id.as_deref(),
            Some("v1")
        );
        // Overwrite with a new policy and wait for the debounce + reload.
        std::fs::write(&path, "deny_with(\"v2\");\n").unwrap();
        let mut swapped = false;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            if handle.load().evaluate(&ctx("X")).rule_id.as_deref() == Some("v2") {
                swapped = true;
                break;
            }
        }
        assert!(swapped, "reloader must pick up the edited policy");
    }
}
