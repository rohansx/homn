//! The capture-daemon runtime — ties a [`Source`](homn_sources::Source) (ScreenpipeTail) →
//! [`Gate`](homn_gate::Gate) → [`Store`] → the [`ControlState`] + control socket, so `homn
//! capture daemon` actually ingests (T028). The drain loop respects `ControlState::is_paused()`
//! (set by `homn capture pause`) and reports counters back to it (surfaced by `homn status`).
//!
//! The store is `AgidbStore` when a brain path is given (needs `brain-agidb`) or `MemoryStore`
//! otherwise (ephemeral — useful for a smoke run, not for real persistence). The audit [`Db`]
//! holds the watermarks + decision receipts so a crash re-reads from the last confirmed position.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use homn_audit::Db;
use homn_gate::{Gate, IngestPolicy};
use homn_sources::{ScreenpipeTail, Source};

use crate::control::{ControlServer, ControlState};
use crate::pipeline::{Pipeline, TickResult};
use crate::store::{MemoryStore, Store};

#[cfg(feature = "brain-agidb")]
use crate::store::AgidbStore;

/// Where the daemon finds its inputs + writes its state.
#[derive(Debug, Clone)]
pub struct CaptureDaemonConfig {
    /// Path to Screenpipe's `db.sqlite` (tailed by `ScreenpipeTail`).
    pub screenpipe_db: PathBuf,
    /// Path to an agidb brain directory. `Some` → `AgidbStore` (needs `brain-agidb`); `None` →
    /// ephemeral `MemoryStore` (ingests are lost on restart — fine for a smoke run).
    pub brain_path: Option<PathBuf>,
    /// Path to `policies/ingest.rhai`. Loaded if it exists; a permissive default is used otherwise.
    pub ingest_policy_path: PathBuf,
    /// Path to the homn audit DB (watermarks + decision receipts).
    pub audit_db_path: PathBuf,
    /// Path for the control socket (`$XDG_RUNTIME_DIR/homnd.sock`).
    pub socket_path: PathBuf,
}

/// Run the capture daemon until fatally interrupted: builds the source/gate/store/pipeline,
/// spawns the drain loop, and serves the control socket on the main task.
pub async fn run_capture_daemon(cfg: CaptureDaemonConfig) -> anyhow::Result<()> {
    // 1. Ingest policy → gate.
    let policy = if cfg.ingest_policy_path.exists() {
        IngestPolicy::load(&cfg.ingest_policy_path)?
    } else {
        tracing::warn!(
            policy = %cfg.ingest_policy_path.display(),
            "ingest policy not found; using a permissive default"
        );
        IngestPolicy::compile(
            "allow(); // permissive default (no policy file — always-on secrets scan still runs)",
        )?
    };
    let gate = Gate::new(policy);

    // 2. Store: agidb if a brain path is given (feature-gated), else in-memory.
    let store: Arc<dyn Store> = open_store(&cfg.brain_path).await?;

    // 3. Audit DB (watermarks + receipts).
    if let Some(parent) = cfg.audit_db_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let db = Db::open(&cfg.audit_db_path).await?;

    // 4. Source + pipeline.
    let source = ScreenpipeTail::new(&cfg.screenpipe_db);
    let pipeline = Arc::new(Pipeline::new(source.id().to_owned(), gate, store, db));

    // 5. Control state + drain loop. The loop checks `is_paused()` each pass (set by
    //    `homn capture pause`), drains the source through the pipeline, and mirrors stats
    //    into the control state so `homn status` reports them.
    let control = ControlState::new();
    let drain_control = control.clone();
    let drain_pipeline = pipeline.clone();
    let drain_task = tokio::spawn(async move {
        if let Err(err) = drain_loop(&drain_pipeline, &source, &drain_control).await {
            tracing::error!(error = %err, "homnd drain loop exited with error");
        }
    });

    // 6. Control socket on the main task.
    let server = ControlServer::bind(&cfg.socket_path)
        .await
        .map_err(|e| anyhow::anyhow!("bind {}: {e}", cfg.socket_path.display()))?;
    tracing::info!(
        socket = %cfg.socket_path.display(),
        screenpipe_db = %cfg.screenpipe_db.display(),
        brain = ?cfg.brain_path,
        "homnd capture daemon ready"
    );
    eprintln!(
        "homnd ready on {} (tailing {})",
        cfg.socket_path.display(),
        cfg.screenpipe_db.display()
    );
    let serve_result = server.serve(control).await;
    drain_task.abort();
    serve_result?;
    Ok(())
}

/// The per-source drain loop: drain while there's data, back off when exhausted, and respect
/// the control-state pause flag.
async fn drain_loop(
    pipeline: &Pipeline,
    source: &dyn Source,
    control: &ControlState,
) -> anyhow::Result<()> {
    loop {
        if control.is_paused() {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }
        match pipeline.tick(source).await? {
            TickResult::Paused => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            TickResult::Exhausted => {
                // Nothing new right now — mirror stats and back off briefly.
                control.set_stats(pipeline.stats().await).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            TickResult::Drained { .. } => {
                // Keep draining while there's data; mirror stats each pass.
                control.set_stats(pipeline.stats().await).await;
            }
        }
    }
}

/// Build the store. `Some(path)` → `AgidbStore` (needs `brain-agidb`); `None` → `MemoryStore`.
async fn open_store(brain_path: &Option<PathBuf>) -> anyhow::Result<Arc<dyn Store>> {
    let Some(path) = brain_path.as_ref() else {
        tracing::warn!("no --brain given; using ephemeral MemoryStore (ingests lost on restart)");
        return Ok(Arc::new(MemoryStore::default()));
    };
    #[cfg(feature = "brain-agidb")]
    {
        let brain = agidb::Agidb::open_with(
            agidb::AgidbConfig::new(path).with_extractor(agidb::ExtractorSetup::Null),
        )
        .await
        .map_err(|e| anyhow::anyhow!("open brain {}: {e}", path.display()))?;
        Ok(Arc::new(AgidbStore::new(Arc::new(brain))))
    }
    #[cfg(not(feature = "brain-agidb"))]
    {
        anyhow::bail!(
            "`homn capture daemon --brain {}` needs the agidb brain. Rebuild with:\n  \
             cargo build -p homn-bin --features brain-agidb",
            path.display()
        );
    }
}
