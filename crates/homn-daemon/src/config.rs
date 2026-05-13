//! Daemon configuration loaded from `homn.toml`.
//!
//! Defaults if the file is missing or fields are absent — never errors on a missing config,
//! only on a malformed one. See
//! [`specs/001-policy-engine/data-model.md`](../../../specs/001-policy-engine/data-model.md)
//! §"On-disk policy state" for the canonical schema.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level daemon configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    /// `[daemon]` section.
    pub daemon: DaemonSection,
    /// `[audit]` section.
    pub audit: AuditSection,
    /// `[learning]` section.
    pub learning: LearningSection,
    /// `[policy]` section.
    pub policy: PolicySection,
    /// `[hook]` section.
    pub hook: HookSection,
    /// `[pty_wrapper]` section.
    pub pty_wrapper: PtyWrapperSection,
    /// `[surfaces]` section.
    pub surfaces: SurfacesSection,
    /// `[mcp]` section.
    pub mcp: McpSection,
}


/// `[daemon]` — socket paths, shutdown grace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonSection {
    /// Path to the request-response Unix socket. Default: `$XDG_RUNTIME_DIR/homn.sock`.
    pub socket_path: PathBuf,
    /// Path to the event-broadcast socket. Default: `$XDG_RUNTIME_DIR/homn-events.sock`.
    pub events_socket_path: PathBuf,
    /// Time the daemon waits for in-flight requests before forcing shutdown.
    pub shutdown_grace_ms: u32,
}

impl Default for DaemonSection {
    fn default() -> Self {
        Self {
            socket_path: default_runtime_dir().join("homn.sock"),
            events_socket_path: default_runtime_dir().join("homn-events.sock"),
            shutdown_grace_ms: 2_000,
        }
    }
}

/// `[audit]` — SQLite path + retention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AuditSection {
    /// Path to `audit.db`.
    pub db_path: PathBuf,
    /// How many days of audit data to keep; `0` = keep forever.
    pub retention_days: u32,
    /// Hour-of-day at which the daily compaction job runs (0–23, local time).
    pub compaction_hour: u8,
}

impl Default for AuditSection {
    fn default() -> Self {
        Self {
            db_path: default_data_dir().join("homn").join("audit.db"),
            retention_days: 30,
            compaction_hour: 3,
        }
    }
}

/// `[learning]` — pattern detection thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearningSection {
    /// Path to `learning.db`.
    pub db_path: PathBuf,
    /// Consecutive same-answer asks needed to fire a suggestion.
    pub threshold: u32,
    /// How long a rejected suggestion stays silenced.
    pub snooze_days: u32,
}

impl Default for LearningSection {
    fn default() -> Self {
        Self {
            db_path: default_data_dir().join("homn").join("learning.db"),
            threshold: 5,
            snooze_days: 30,
        }
    }
}

/// `[policy]` — Rhai engine budgets + policies directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicySection {
    /// Directory containing `.rhai` policy files.
    pub policies_dir: PathBuf,
    /// Per-rule wall-clock budget (ms).
    pub per_rule_budget_ms: u32,
    /// Per-call total wall-clock budget across all rules (ms).
    pub per_call_budget_ms: u32,
    /// Maximum Rhai engine operations per evaluation.
    pub max_operations: u64,
}

impl Default for PolicySection {
    fn default() -> Self {
        Self {
            policies_dir: default_config_dir().join("homn").join("policies"),
            per_rule_budget_ms: 50,
            per_call_budget_ms: 200,
            max_operations: 100_000,
        }
    }
}

/// `[hook]` — hook subcommand behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HookSection {
    /// Total hook timeout (must be under Claude Code's hook timeout, default 30 s).
    pub timeout_ms: u32,
    /// What to return if the daemon errors or times out: `"ask"` is the safe default.
    pub fallback_decision: String,
}

impl Default for HookSection {
    fn default() -> Self {
        Self {
            timeout_ms: 28_000,
            fallback_decision: "ask".to_owned(),
        }
    }
}

/// `[pty_wrapper]` — `homn run claude` configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PtyWrapperSection {
    /// Whether the PTY wrapper is allowed to spawn at all.
    pub enabled: bool,
    /// Regex matching Claude Code's permission prompt in the PTY stream.
    pub prompt_regex: String,
    /// Time window in which a daemon-decided `deny` will be synthesized as `n\n`.
    pub deny_race_window_ms: u32,
}

impl Default for PtyWrapperSection {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt_regex: r"Do you want to proceed\? \(y/n\):".to_owned(),
            deny_race_window_ms: 200,
        }
    }
}

/// `[surfaces]` — which surfaces are enabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfacesSection {
    /// Default surface: `"tui"`, `"face"`, or `"auto"`.
    pub default: String,
    /// Whether the Tauri face autostarts. v1 default: `false` (opt-in).
    pub face_enabled: bool,
    /// ntfy topic for phone push (empty = disabled).
    pub ntfy_topic: String,
    /// Minutes of idle before ntfy mirroring kicks in.
    pub ntfy_after_idle_minutes: u32,
}

impl Default for SurfacesSection {
    fn default() -> Self {
        Self {
            default: "tui".to_owned(),
            face_enabled: false,
            ntfy_topic: String::new(),
            ntfy_after_idle_minutes: 5,
        }
    }
}

/// `[mcp]` — MCP server transports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSection {
    /// Whether stdio transport is enabled.
    pub stdio_enabled: bool,
    /// Whether the HTTP transport is enabled.
    pub http_enabled: bool,
    /// HTTP bind address (only used when `http_enabled` is `true`).
    pub http_bind: String,
}

impl Default for McpSection {
    fn default() -> Self {
        Self {
            stdio_enabled: true,
            http_enabled: false,
            http_bind: "127.0.0.1:9874".to_owned(),
        }
    }
}

/// Canonical config path under `$XDG_CONFIG_HOME/homn/homn.toml`.
pub fn default_config_path() -> PathBuf {
    default_config_dir().join("homn").join("homn.toml")
}

/// Load the daemon config from a TOML file. Missing file → defaults. Malformed file → error.
pub fn load_config(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let path = path.as_ref();
    if !path.exists() {
        tracing::info!(path = %path.display(), "no homn.toml found; using defaults");
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path)?;
    let cfg: Config = toml::from_str(&text)?;
    Ok(cfg)
}

fn default_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn default_data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg);
    }
    home_dir()
        .map(|h| h.join(".local").join("share"))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

fn default_config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    home_dir()
        .map(|h| h.join(".config"))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_config_yields_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.daemon.shutdown_grace_ms, 2_000);
        assert_eq!(cfg.policy.per_rule_budget_ms, 50);
        assert_eq!(cfg.policy.per_call_budget_ms, 200);
        assert!(!cfg.surfaces.face_enabled);
        assert!(cfg.mcp.stdio_enabled);
    }

    #[test]
    fn partial_overrides_apply() {
        let toml_src = r#"
            [policy]
            per_call_budget_ms = 500

            [surfaces]
            face_enabled = true
        "#;
        let cfg: Config = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.policy.per_call_budget_ms, 500);
        // Other policy fields should still have their defaults:
        assert_eq!(cfg.policy.per_rule_budget_ms, 50);
        assert!(cfg.surfaces.face_enabled);
        // Defaults outside the overridden sections survive:
        assert_eq!(cfg.daemon.shutdown_grace_ms, 2_000);
    }

    #[test]
    fn malformed_toml_errors() {
        let bad = "this is not toml = = = =";
        let err: Result<Config, _> = toml::from_str(bad);
        assert!(err.is_err());
    }

    #[test]
    fn missing_file_returns_defaults() {
        let nowhere = PathBuf::from("/definitely/does/not/exist/homn.toml");
        let cfg = load_config(&nowhere).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn malformed_file_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("homn.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "garbage = = =").unwrap();
        assert!(load_config(&path).is_err());
    }
}
