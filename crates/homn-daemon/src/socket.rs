//! Unix-socket JSON-line RPC server for the daemon.
//!
//! T013 deliverable: listens at `$XDG_RUNTIME_DIR/homn.sock`, accepts connections, reads JSON
//! lines per [`hook-protocol.md`](../../../../specs/001-policy-engine/contracts/hook-protocol.md),
//! and dispatches to a stub `ping` handler that returns `{"pong": true}`.
//!
//! Real method dispatch (decisions, policies, learning, surfaces) lands in T030 and after.

use std::path::{Path, PathBuf};

use homn_types::{ErrorObject, Request, Response};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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
            // Remove a stale socket from a previous run. Safe because Unix sockets are not
            // file-locked while live.
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

    /// Accept connections forever, handling each with the built-in `ping` stub. Returns only on a
    /// fatal accept error.
    ///
    /// Wraps each connection in its own Tokio task so slow clients don't block accept.
    pub async fn serve_pings_forever(self) -> anyhow::Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            tokio::spawn(async move {
                if let Err(err) = handle_connection(stream).await {
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

async fn handle_connection(stream: UnixStream) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(&line);
        let mut s = serde_json::to_string(&response)?;
        s.push('\n');
        write_half.write_all(s.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Public for testing — dispatches one JSON-line request to a stub handler.
pub fn handle_line(line: &str) -> Response {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(err) => {
            return Response::err(
                "",
                ErrorObject::new("invalid_frame", err.to_string()),
            );
        }
    };

    match req.method.as_str() {
        "ping" => Response::ok(req.id, json!({"pong": true})),
        other => Response::err(
            req.id,
            ErrorObject::new("unknown_method", format!("method `{other}` is not implemented yet")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn ping_returns_pong() {
        let req = json!({"id": "01H", "method": "ping", "params": {}}).to_string();
        let resp = handle_line(&req);
        match resp {
            Response::Ok { id, result } => {
                assert_eq!(id, "01H");
                assert_eq!(result["pong"], true);
            }
            Response::Err { .. } => panic!("expected Ok"),
        }
    }

    #[test]
    fn unknown_method_returns_error() {
        let req = json!({"id": "02H", "method": "nope", "params": {}}).to_string();
        let resp = handle_line(&req);
        match resp {
            Response::Err { id, error } => {
                assert_eq!(id, "02H");
                assert_eq!(error.code, "unknown_method");
            }
            Response::Ok { .. } => panic!("expected Err"),
        }
    }

    #[test]
    fn invalid_frame_returns_error_with_empty_id() {
        let resp = handle_line("not json");
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
        tokio::spawn(async move {
            let _ = server.serve_pings_forever().await;
        });

        // Give the listener a moment to come up.
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
