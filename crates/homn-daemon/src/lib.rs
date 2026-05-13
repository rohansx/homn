//! The long-running `homn` daemon.
//!
//! Owns the event bus, the Unix-socket request-response listener (`homn.sock`), the policy
//! engine + ruleset, and the audit DB handle. T013/T014/T015/T016/T026/T028/T030 are live; the
//! human-mediated `Ask` round-trip (T032/T033) and event-broadcast socket (T014b) follow.
//!
//! See [`docs/architecture/overview.md`](../../../docs/architecture/overview.md) and
//! [`specs/001-policy-engine/contracts/`](../../../specs/001-policy-engine/contracts/).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

pub mod config;
pub mod event_bus;
pub mod handler;
pub mod socket;

pub use config::{load_config, Config};
pub use event_bus::{EventBus, Subscriber};
pub use handler::DaemonState;
pub use socket::SocketServer;

use homn_audit::Db;
use homn_policy::{Engine, Reloader, RuleSet};

/// Open the audit DB. Convenience for both `run()` and tests.
async fn open_audit(config: &Config) -> anyhow::Result<Arc<Db>> {
    if let Some(parent) = config.audit.db_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    tracing::info!(path = %config.audit.db_path.display(), "opening audit DB");
    Ok(Arc::new(Db::open(&config.audit.db_path).await?))
}

/// Build a [`DaemonState`] without a file-watcher. The ruleset is loaded once from disk; you'll
/// need to restart the daemon to pick up changes. Used by tests and as the simple code path.
pub async fn build_state(config: &Config) -> anyhow::Result<DaemonState> {
    let engine = Engine::new();
    let default_policy = config.policy.policies_dir.join("default.rhai");
    let rules = if default_policy.exists() {
        tracing::info!(path = %default_policy.display(), "loading default policy");
        homn_policy::load_ruleset(&default_policy)?
    } else {
        tracing::info!(
            path = %default_policy.display(),
            "no default policy file; starting with an empty ruleset (every request → ask)"
        );
        RuleSet::parse(&engine, "", "default.rhai")?
    };
    let audit = open_audit(config).await?;
    Ok(DaemonState::with_static_rules(engine, rules, audit))
}

/// Build a [`DaemonState`] *with* a live file-watcher attached when the policy file exists.
/// Returns the state plus an optional [`Reloader`] that the caller must keep alive for the
/// watcher to remain active. Implements T026 hot-reload.
pub async fn build_state_with_reloader(
    config: &Config,
) -> anyhow::Result<(DaemonState, Option<Reloader>)> {
    let engine = Engine::new();
    let default_policy = config.policy.policies_dir.join("default.rhai");

    let (rules_handle, reloader) = if default_policy.exists() {
        tracing::info!(path = %default_policy.display(), "loading default policy with hot reload");
        let reloader = homn_policy::spawn_reloader(engine.clone(), &default_policy)?;
        (reloader.handle.clone(), Some(reloader))
    } else {
        tracing::info!(
            path = %default_policy.display(),
            "no default policy file; starting with an empty ruleset (every request → ask)"
        );
        let empty = RuleSet::parse(&engine, "", "default.rhai")?;
        let handle: homn_policy::RuleSetHandle = Arc::new(arc_swap::ArcSwap::from_pointee(empty));
        (handle, None)
    };

    let audit = open_audit(config).await?;
    let state = DaemonState {
        engine,
        rules: rules_handle,
        audit,
    };
    Ok((state, reloader))
}

/// Run the daemon to completion using the given config.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let (state, _reloader) = build_state_with_reloader(&config).await?;

    let socket = SocketServer::bind(&config.daemon.socket_path).await?;
    tracing::info!(
        socket = %config.daemon.socket_path.display(),
        "homn daemon listening"
    );

    // `_reloader` is kept in scope to keep the file watcher alive for the lifetime of `run`.
    socket.serve(state).await
}
