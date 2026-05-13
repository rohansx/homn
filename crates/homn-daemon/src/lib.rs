//! The long-running `homn` daemon.
//!
//! Owns the event bus, the Unix-socket request-response listener (`homn.sock`), the policy
//! engine + ruleset, and the audit DB handle. T013/T014/T015/T016/T028/T030 are all live; the
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
use homn_policy::{Engine, RuleSet};

/// Build the daemon's [`DaemonState`] by opening the audit DB and loading the default ruleset
/// from `<policy.policies_dir>/default.rhai`. A missing policy file yields an empty ruleset
/// (every request falls through to the default `Ask`).
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

    if let Some(parent) = config.audit.db_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    tracing::info!(path = %config.audit.db_path.display(), "opening audit DB");
    let audit = Arc::new(Db::open(&config.audit.db_path).await?);

    Ok(DaemonState {
        engine,
        rules: Arc::new(rules),
        audit,
    })
}

/// Run the daemon to completion using the given config.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let state = build_state(&config).await?;

    let socket = SocketServer::bind(&config.daemon.socket_path).await?;
    tracing::info!(
        socket = %config.daemon.socket_path.display(),
        "homn daemon listening"
    );

    socket.serve(state).await
}
