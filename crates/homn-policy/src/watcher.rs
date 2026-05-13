//! Hot-reload watcher for policy files (T026).
//!
//! Watches the *parent directory* of the policy file (because most editors write via
//! atomic rename, which changes the inode of the named file), and atomically swaps the
//! shared [`ArcSwap<RuleSet>`] when the file changes.
//!
//! Failure model: a syntactically broken policy file does NOT crash the daemon and does NOT
//! evict the previously-loaded ruleset. We log a warning at WARN level and keep serving the
//! last-good version until the file is fixed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{Engine, RuleSet};

/// A live, hot-reloading ruleset. Cheap to clone (Arc); cheap to read (lock-free).
///
/// `RuleSetHandle::load()` returns the current Arc<RuleSet> for evaluation.
pub type RuleSetHandle = Arc<ArcSwap<RuleSet>>;

/// Background reloader returned from [`spawn_reloader`]. Drop to stop watching.
pub struct Reloader {
    /// The current ruleset, atomically swappable.
    pub handle: RuleSetHandle,
    /// The platform watcher; kept alive for the lifetime of the reloader.
    _watcher: RecommendedWatcher,
}

impl std::fmt::Debug for Reloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reloader")
            .field("handle", &"<live RuleSetHandle>")
            .finish_non_exhaustive()
    }
}

/// Load the policy file at `path` and start watching its parent directory for changes.
/// Returns a [`Reloader`] holding both the live `RuleSetHandle` and the underlying watcher.
///
/// If the initial load fails, returns the error and starts nothing. If a *subsequent* reload
/// fails (broken syntax, deleted file), the watcher logs a warning and keeps the prior ruleset
/// active.
pub fn spawn_reloader(engine: Engine, path: impl Into<PathBuf>) -> anyhow::Result<Reloader> {
    let path = path.into();
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone());
    let parent = canonical
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let initial = RuleSet::load(&engine, &canonical)?;
    let handle: RuleSetHandle = Arc::new(ArcSwap::from_pointee(initial));

    let handle_for_watcher = handle.clone();
    let path_for_watcher = canonical.clone();
    let engine_for_watcher = engine.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let event = match res {
            Ok(ev) => ev,
            Err(err) => {
                tracing::warn!(error = %err, "policy file-watch error");
                return;
            }
        };

        if !is_relevant(&event) {
            return;
        }
        if !event.paths.iter().any(|p| same_path(p, &path_for_watcher)) {
            return;
        }

        // Tiny debounce: editors often emit multiple events back-to-back (Modify(Name)
        // + Create + Modify(Data)). Sleep briefly so the file is fully written before
        // we parse it.
        std::thread::sleep(Duration::from_millis(20));

        match RuleSet::load(&engine_for_watcher, &path_for_watcher) {
            Ok(new) => {
                let summary = format!(
                    "deny={} ask={} allow={}",
                    new.deny_rules().count(),
                    new.ask_rules().count(),
                    new.allow_rules().count(),
                );
                handle_for_watcher.store(Arc::new(new));
                tracing::info!(path = %path_for_watcher.display(), %summary, "policy hot-reloaded");
            }
            Err(err) => {
                tracing::warn!(
                    path = %path_for_watcher.display(),
                    error = %err,
                    "policy reload failed; keeping previous ruleset"
                );
            }
        }
    })?;

    // Watching the parent directory rather than the file itself is more reliable across
    // editors that use atomic rename (vim, helix, vscode).
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;

    tracing::info!(path = %canonical.display(), "watching policy file");
    Ok(Reloader {
        handle,
        _watcher: watcher,
    })
}

fn is_relevant(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    )
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || a.canonicalize().ok().as_deref() == Some(b)
        || b.canonicalize().ok().as_deref() == Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::Decision;
    use std::time::Instant;

    fn req(tool: &str, cmd: &str) -> crate::EvalRequest {
        crate::EvalRequest {
            tool: tool.into(),
            cmd: cmd.into(),
            home: "/home/rsx".into(),
            cwd: "/home/rsx/dev".into(),
            ..Default::default()
        }
    }

    #[test]
    fn initial_load_failure_propagates() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.rhai");
        let err = spawn_reloader(Engine::new(), &missing).unwrap_err();
        assert!(
            format!("{err}").to_lowercase().contains("io"),
            "expected IO error, got: {err}"
        );
    }

    #[test]
    fn reload_after_edit_picks_up_new_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("default.rhai");
        std::fs::write(&path, r#"allow if tool == "Read";"#).unwrap();
        let engine = Engine::new();
        let reloader = spawn_reloader(engine.clone(), &path).unwrap();

        // Initial: Read is allowed, Bash unmatched (default ask)
        let initial = reloader.handle.load();
        assert_eq!(engine.eval(&initial, &req("Read", "")).decision, Decision::Allow);
        assert_eq!(engine.eval(&initial, &req("Bash", "rm")).decision, Decision::Ask);

        // Rewrite the file with a new ruleset.
        std::fs::write(
            &path,
            r#"
                deny if tool == "Bash" && cmd.contains("rm -rf");
                allow if tool == "Read";
            "#,
        )
        .unwrap();

        // Wait up to 2s for the watcher to pick up.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let current = reloader.handle.load();
            if engine.eval(&current, &req("Bash", "rm -rf /home/rsx/foo")).decision
                == Decision::Deny
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watcher did not pick up the edit within 2s"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        // Reread to confirm:
        let after = reloader.handle.load();
        assert_eq!(
            engine.eval(&after, &req("Bash", "rm -rf /home/rsx/foo")).decision,
            Decision::Deny
        );
        // Pre-existing rule still in effect:
        assert_eq!(engine.eval(&after, &req("Read", "")).decision, Decision::Allow);
    }

    #[test]
    fn broken_reload_keeps_previous_ruleset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("default.rhai");
        std::fs::write(&path, r#"allow if tool == "Read";"#).unwrap();
        let engine = Engine::new();
        let reloader = spawn_reloader(engine.clone(), &path).unwrap();

        assert_eq!(
            engine
                .eval(&reloader.handle.load(), &req("Read", ""))
                .decision,
            Decision::Allow
        );

        // Replace with broken syntax.
        std::fs::write(&path, "this is not valid rhai = = =").unwrap();

        // Give the watcher time to process; assert previous ruleset is still active.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            engine
                .eval(&reloader.handle.load(), &req("Read", ""))
                .decision,
            Decision::Allow,
            "broken reload should not have evicted the previous ruleset"
        );
    }
}
