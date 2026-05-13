//! Unix-socket JSON-line RPC server for the daemon.
//!
//! T013 deliverable: listens at `$XDG_RUNTIME_DIR/homn.sock`, accepts connections, reads JSON
//! lines per [`hook-protocol.md`](../../../../specs/001-policy-engine/contracts/hook-protocol.md),
//! and dispatches via [`crate::handler::dispatch`]. The dispatch function consults the policy
//! engine and writes to the audit log; surface-mediated `Ask` is wired in T032/T033.

use std::path::{Path, PathBuf};

use homn_types::{ErrorObject, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::handler::{dispatch, DaemonState};

/// A bound, listening JSON-line RPC server.
pub struct SocketServer {
    listener: UnixListener,
    path: PathBuf,
}

impl SocketServer {
    /// Bind a `SocketServer` at the given path. Removes any stale socket file at that path.
    pub async fn bind(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let listener = UnixListener::bind(&path)?;
        Ok(Self { listener, path })
    }

    /// Borrow the socket path the server is bound to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections forever, dispatching each line via [`dispatch`] against the given
    /// [`DaemonState`]. Returns only on a fatal accept error.
    pub async fn serve(self, state: DaemonState) -> anyhow::Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_connection(stream, state).await {
                    tracing::warn!(error = %err, "connection error");
                }
            });
        }
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn handle_connection(stream: UnixStream, state: DaemonState) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(&line, &state).await;
        let mut s = serde_json::to_string(&response)?;
        s.push('\n');
        write_half.write_all(s.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Public for testing — dispatches one JSON-line request through the daemon state.
pub async fn handle_line(line: &str, state: &DaemonState) -> Response {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(err) => {
            return Response::err("", ErrorObject::new("invalid_frame", err.to_string()));
        }
    };
    dispatch(state, req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::DaemonState;
    use homn_audit::Db;
    use homn_policy::{Engine, RuleSet};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn empty_state() -> DaemonState {
        let engine = Engine::new();
        let rules = RuleSet::parse(&engine, "", "default.rhai").unwrap();
        let audit = Arc::new(Db::in_memory().await.unwrap());
        DaemonState::with_static_rules(engine, rules, audit)
    }

    #[tokio::test]
    async fn ping_returns_pong() {
        let state = empty_state().await;
        let req = json!({"id": "01H", "method": "ping", "params": {}}).to_string();
        let resp = handle_line(&req, &state).await;
        match resp {
            Response::Ok { id, result } => {
                assert_eq!(id, "01H");
                assert_eq!(result["pong"], true);
            }
            Response::Err { .. } => panic!("expected Ok"),
        }
    }

    #[tokio::test]
    async fn unknown_method_returns_error() {
        let state = empty_state().await;
        let req = json!({"id": "02H", "method": "nope", "params": {}}).to_string();
        let resp = handle_line(&req, &state).await;
        match resp {
            Response::Err { id, error } => {
                assert_eq!(id, "02H");
                assert_eq!(error.code, "unknown_method");
            }
            Response::Ok { .. } => panic!("expected Err"),
        }
    }

    #[tokio::test]
    async fn invalid_frame_returns_error_with_empty_id() {
        let state = empty_state().await;
        let resp = handle_line("not json", &state).await;
        match resp {
            Response::Err { id, error } => {
                assert_eq!(id, "");
                assert_eq!(error.code, "invalid_frame");
            }
            Response::Ok { .. } => panic!("expected Err"),
        }
    }

    #[tokio::test]
    async fn end_to_end_ping_over_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("homn.sock");

        let server = SocketServer::bind(&sock_path).await.unwrap();
        let bound = server.path().to_path_buf();
        let state = empty_state().await;
        tokio::spawn(async move {
            let _ = server.serve(state).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = tokio::net::UnixStream::connect(&bound).await.unwrap();
        let line = format!(
            "{}\n",
            serde_json::to_string(&Request {
                id: "abc".into(),
                method: "ping".into(),
                params: json!({}),
            })
            .unwrap()
        );
        stream.write_all(line.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream).lines();
        let resp_line = reader.next_line().await.unwrap().expect("got response");
        let resp: Response = serde_json::from_str(&resp_line).unwrap();
        match resp {
            Response::Ok { id, result } => {
                assert_eq!(id, "abc");
                assert_eq!(result["pong"], true);
            }
            Response::Err { error, .. } => panic!("unexpected error: {error:?}"),
        }
    }
}
