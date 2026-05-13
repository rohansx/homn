//! The long-running `homn` daemon.
//!
//! Owns the event bus, the Unix-socket request-response listener (`homn.sock`), the event
//! broadcast listener (`homn-events.sock`), and (in later tasks) the policy dispatch path,
//! audit writer, and MCP host.
//!
//! See [`docs/architecture/overview.md`](../../../docs/architecture/overview.md) and
//! [`specs/001-policy-engine/contracts/`](../../../specs/001-policy-engine/contracts/).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod event_bus;
pub mod socket;

pub use config::{load_config, Config};
pub use event_bus::{EventBus, Subscriber};
pub use socket::SocketServer;

/// Run the daemon to completion using the given config.
///
/// Boots the event bus + the two Unix-socket listeners. Returns when the listeners exit (which,
/// in production, only happens on signal-driven shutdown — not yet implemented).
pub async fn run(config: Config) -> anyhow::Result<()> {
    let bus = EventBus::new(1024);

    // Request-response socket
    let request_socket = SocketServer::bind(&config.daemon.socket_path).await?;
    tracing::info!(
        socket = %config.daemon.socket_path.display(),
        "homn daemon listening (request-response)"
    );

    // (Event-broadcast socket lands in a follow-up; T014 just provides the bus type for now.)
    let _ = bus; // silence unused

    request_socket.serve_pings_forever().await
}
