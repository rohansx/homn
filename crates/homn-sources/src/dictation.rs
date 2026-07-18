//! `DictationPipe` — a push-based [`Source`] for convox-voice dictation text (task T026).
//!
//! convox-voice connects to a unix socket (default `$XDG_RUNTIME_DIR/homn-dictation.sock`) and
//! writes newline-delimited UTF-8 utterances. Each line becomes one pre-gate [`RawCapture`] with
//! `kind() = Dictation` and a monotonic line sequence number as its cursor. Lines are buffered
//! in-memory (bounded) so the daemon's `fetch_since(cursor)` loop drains everything strictly
//! after its watermark, including a superset re-read after a crash (at-least-once upstream).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use homn_types::{Cursor, RawCapture, SourceKind, SpeakerTag};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::{Batch, Source, SourceError};

/// Hard cap on buffered lines (backpressure contract: never buffer unbounded upstream data).
/// Dictation is human-paced, so the daemon's fetch loop drains this long before it fills.
// ponytail: oldest lines are evicted past the cap; spill-to-disk only if real overruns show up.
const MAX_BUFFERED_LINES: usize = 4096;

/// One buffered utterance: (sequence number, arrival time, pre-gate text).
struct Buffer {
    /// Last sequence number handed out; 0 = nothing pushed yet.
    seq: u64,
    lines: VecDeque<(u64, DateTime<Utc>, String)>,
}

/// Push-based dictation source. Owns the listening socket for its lifetime; dropping the pipe
/// stops the accept loop and removes the socket file.
pub struct DictationPipe {
    id: String,
    socket_path: PathBuf,
    buffer: Arc<Mutex<Buffer>>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl DictationPipe {
    /// Bind the dictation socket and start accepting writers.
    ///
    /// `socket_path` defaults to `$XDG_RUNTIME_DIR/homn-dictation.sock`; if neither a path nor
    /// `XDG_RUNTIME_DIR` is available this fails closed rather than guessing a world-writable
    /// location. Must be called from within a tokio runtime.
    pub fn bind(id: impl Into<String>, socket_path: Option<PathBuf>) -> Result<Self, SourceError> {
        let socket_path = match socket_path {
            Some(p) => p,
            None => default_socket_path()?,
        };

        // Remove a stale socket file from a previous run; a live listener would have kept it.
        // Ignore NotFound — anything else (perms) will surface as a bind error just below.
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).map_err(|e| {
            SourceError::Unavailable(format!("bind {}: {e}", socket_path.display()))
        })?;

        let buffer = Arc::new(Mutex::new(Buffer {
            seq: 0,
            lines: VecDeque::new(),
        }));

        let accept_buffer = Arc::clone(&buffer);
        let accept_task = tokio::spawn(async move {
            // Connection tasks live in the JoinSet so aborting the accept task aborts them too.
            // ponytail: finished handles are not reaped; convox-voice is a single long-lived
            // client, so the set stays tiny.
            let mut conns = tokio::task::JoinSet::new();
            // An accept error means the listener is broken: stop accepting (fail closed).
            // The daemon's health loop notices the source going quiet and can rebind.
            while let Ok((stream, _)) = listener.accept().await {
                conns.spawn(pump_lines(stream, Arc::clone(&accept_buffer)));
            }
        });

        Ok(Self {
            id: id.into(),
            socket_path,
            buffer,
            accept_task,
        })
    }

    /// The socket path this pipe is listening on.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for DictationPipe {
    fn drop(&mut self) {
        self.accept_task.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[async_trait]
impl Source for DictationPipe {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Dictation
    }

    async fn fetch_since(&self, cursor: Option<&Cursor>) -> Result<Batch, SourceError> {
        // The opaque cursor is our monotonic line sequence number (0 = from the beginning).
        let since = match cursor {
            Some(Cursor(v)) => v
                .as_u64()
                .ok_or_else(|| SourceError::InvalidCursor(v.to_string()))?,
            None => 0,
        };

        let buffer = self
            .buffer
            .lock()
            .map_err(|_| SourceError::Other("dictation buffer lock poisoned".to_owned()))?;

        let items: Vec<RawCapture> = buffer
            .lines
            .iter()
            .filter(|(seq, _, _)| *seq > since)
            .map(|(seq, captured_at, text)| RawCapture {
                upstream_ref: format!("dictation-line-{seq}"),
                source: SourceKind::Dictation,
                app: None,
                captured_at: *captured_at,
                text: text.clone(),
                // Dictation is by definition the user speaking.
                speaker: Some(SpeakerTag::Me),
            })
            .collect();

        // Monotonic: never regress below the input cursor, even if the buffer is behind it.
        // `lines` is seq-ascending, so the newest buffered seq is at the back.
        let next = buffer
            .lines
            .back()
            .map(|(seq, _, _)| *seq)
            .filter(|seq| *seq > since)
            .unwrap_or(since);

        Ok(Batch {
            items,
            next: Cursor::new(next),
            // Push-based: each fetch drains everything buffered so far.
            exhausted: true,
        })
    }
}

/// Default socket path: `$XDG_RUNTIME_DIR/homn-dictation.sock`. Fails closed if unset.
fn default_socket_path() -> Result<PathBuf, SourceError> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        SourceError::Unavailable(
            "XDG_RUNTIME_DIR not set and no socket path given for DictationPipe".to_owned(),
        )
    })?;
    Ok(PathBuf::from(dir).join("homn-dictation.sock"))
}

/// Read newline-delimited UTF-8 lines from one writer connection into the shared buffer.
/// Invalid UTF-8 or any read error drops the connection (fail closed); the writer reconnects.
async fn pump_lines(stream: UnixStream, buffer: Arc<Mutex<Buffer>>) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        // Lock is never held across an await.
        let Ok(mut buf) = buffer.lock() else { return };
        buf.seq += 1;
        let seq = buf.seq;
        buf.lines.push_back((seq, Utc::now(), line));
        if buf.lines.len() > MAX_BUFFERED_LINES {
            buf.lines.pop_front();
        }
    }
}
