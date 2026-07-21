//! The homnd control socket — a JSON-line RPC over a Unix socket that the `homn capture`
//! CLI talks to (T029). Ops: `status`, `pause`, `resume`. The daemon binds
//! `$XDG_RUNTIME_DIR/homnd.sock`; clients are [`ControlClient`].
//!
//! This is the control surface only — it owns a [`ControlState`] (a paused flag + shared
//! [`PipelineStats`] + a start timestamp). The actual ingestion pipeline (sources → gate →
//! store) is driven by [`crate::pipeline`] and consults `ControlState::is_paused()` before
//! each fetch; this module just exposes the flag + stats over the socket. Real sources
//! (ScreenpipeTail, T025) plug into the pipeline later — the control protocol is stable now.

#![forbid(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::pipeline::PipelineStats;

/// The default control-socket path: `$XDG_RUNTIME_DIR/homnd.sock` (or `/tmp/homnd.sock`).
pub fn default_socket_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("homnd.sock")
}

/// Shared daemon state the control server reads/mutates. Cheap to clone via `Arc`.
#[derive(Clone)]
pub struct ControlState {
    paused: Arc<AtomicBool>,
    started_at: DateTime<Utc>,
    stats: Arc<tokio::sync::Mutex<PipelineStats>>,
}

impl ControlState {
    /// New state, not paused, zeroed stats, started now.
    pub fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            started_at: Utc::now(),
            stats: Arc::new(tokio::sync::Mutex::new(PipelineStats::default())),
        }
    }

    /// Whether capture is paused (the pipeline checks this before each fetch).
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// Pause capture (stop fetching). Idempotent.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    /// Resume capture. Idempotent.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    /// Snapshot the pipeline stats (for `status`).
    pub async fn stats(&self) -> PipelineStats {
        *self.stats.lock().await
    }

    /// Update the pipeline stats (the pipeline calls this periodically).
    pub async fn set_stats(&self, stats: PipelineStats) {
        *self.stats.lock().await = stats;
    }

    /// When the daemon started.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
}

impl Default for ControlState {
    fn default() -> Self {
        Self::new()
    }
}

/// One control operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp {
    /// Report daemon state + counters.
    Status,
    /// Pause capture (stop fetching; the daemon stays up).
    Pause,
    /// Resume capture from pause.
    Resume,
}

/// A control request: `{"op":"status"|"pause"|"resume"}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    /// The operation to perform.
    pub op: ControlOp,
}

/// A control response. `running` is always true (the daemon is up if it answered); `paused`
/// reflects the flag; `stats` is the pipeline snapshot; `started_at` is ISO-8601 UTC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlResponse {
    /// Whether the request succeeded.
    pub ok: bool,
    /// Always true if the daemon answered (it's up).
    pub running: bool,
    /// Whether capture is currently paused.
    pub paused: bool,
    /// When the daemon started (ISO-8601 UTC).
    pub started_at: String,
    /// The pipeline counter snapshot.
    pub stats: PipelineStats,
    /// Present only on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            running: true,
            paused: false,
            started_at: Utc::now().to_rfc3339(),
            stats: PipelineStats::default(),
            error: Some(msg.into()),
        }
    }
}

/// A bound control-socket server.
pub struct ControlServer {
    listener: UnixListener,
    path: PathBuf,
}

impl ControlServer {
    /// Bind at `path`, removing any stale socket file first.
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
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

    /// The path the server is bound to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections forever, dispatching each line via [`handle_line`] against `state`.
    /// Returns only on a fatal accept error.
    pub async fn serve(self, state: ControlState) -> io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_connection(stream, state).await {
                    tracing::warn!(error = %err, "homnd control connection error");
                }
            });
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn handle_connection(stream: UnixStream, state: ControlState) -> io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(&line, &state).await;
        let mut s = serde_json::to_string(&response).map_err(io::Error::other)?;
        s.push('\n');
        write_half.write_all(s.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Dispatch one JSON-line request. Public for testing.
pub async fn handle_line(line: &str, state: &ControlState) -> ControlResponse {
    let req: ControlRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(err) => return ControlResponse::err(format!("invalid request: {err}")),
    };
    match req.op {
        ControlOp::Status => ControlResponse {
            ok: true,
            running: true,
            paused: state.is_paused(),
            started_at: state.started_at().to_rfc3339(),
            stats: state.stats().await,
            error: None,
        },
        ControlOp::Pause => {
            state.pause();
            ControlResponse {
                ok: true,
                running: true,
                paused: true,
                started_at: state.started_at().to_rfc3339(),
                stats: state.stats().await,
                error: None,
            }
        }
        ControlOp::Resume => {
            state.resume();
            ControlResponse {
                ok: true,
                running: true,
                paused: false,
                started_at: state.started_at().to_rfc3339(),
                stats: state.stats().await,
                error: None,
            }
        }
    }
}

/// A client for the control socket — used by `homn capture pause|start|status`.
pub struct ControlClient;

impl ControlClient {
    /// Send `op` to the daemon at `path` and read one response line.
    pub async fn request(path: impl AsRef<Path>, op: ControlOp) -> anyhow::Result<ControlResponse> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path).await.map_err(|e| {
            anyhow::anyhow!(
                "connect {}: {e} (is the daemon running? try `homn capture daemon`)",
                path.display()
            )
        })?;
        let (read_half, mut write_half) = stream.into_split();
        let req = serde_json::to_string(&ControlRequest { op })?;
        write_half.write_all(req.as_bytes()).await?;
        write_half.write_all(b"\n").await?;
        write_half.flush().await?;
        let mut reader = BufReader::new(read_half).lines();
        let line = reader
            .next_line()
            .await
            .map_err(|e| anyhow::anyhow!("read response: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("daemon closed the connection without responding"))?;
        let resp: ControlResponse =
            serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("parse response: {e}"))?;
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_pause_resume_round_trip() {
        let state = ControlState::new();
        assert!(!state.is_paused());

        let status = handle_line(r#"{"op":"status"}"#, &state).await;
        assert!(status.ok && status.running && !status.paused);

        let paused = handle_line(r#"{"op":"pause"}"#, &state).await;
        assert!(paused.ok && paused.paused);
        assert!(state.is_paused(), "pause flag is set");

        let resumed = handle_line(r#"{"op":"resume"}"#, &state).await;
        assert!(resumed.ok && !resumed.paused);
        assert!(!state.is_paused(), "pause flag is cleared");
    }

    #[tokio::test]
    async fn invalid_request_is_an_error_response() {
        let state = ControlState::new();
        let resp = handle_line("not json", &state).await;
        assert!(!resp.ok);
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn client_server_end_to_end() {
        let dir = std::env::temp_dir().join(format!(
            "homnd-control-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("homnd.sock");

        let state = ControlState::new();
        let server = ControlServer::bind(&sock).await.unwrap();
        let serve_state = state.clone();
        let task = tokio::spawn(async move { server.serve(serve_state).await });

        // Pause via the client.
        let r = ControlClient::request(&sock, ControlOp::Pause)
            .await
            .unwrap();
        assert!(r.paused);
        // Status via the client.
        let s = ControlClient::request(&sock, ControlOp::Status)
            .await
            .unwrap();
        assert!(s.paused, "status reflects the pause");
        // Resume.
        let r = ControlClient::request(&sock, ControlOp::Resume)
            .await
            .unwrap();
        assert!(!r.paused);

        task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
