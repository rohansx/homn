//! `homn` binary entry point.
//!
//! Subcommand dispatch via `clap` derive. Each subcommand is a thin wrapper that calls into one
//! of the lib crates (`homn-daemon`, `homn-hook`, `homn-tui`, etc.).
//!
//! T001 (this file): skeleton dispatcher with `--version` and `--help` working; subcommand stubs
//! that print a "not yet implemented in T0XX" message and exit non-zero.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

/// homn — the homunculus for your coding agents.
#[derive(Parser, Debug)]
#[command(
    name = "homn",
    author,
    version,
    about = "Local-first policy daemon, ASCII face, and context graph for coding agents.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the homn daemon (long-running process). T013 implements; today: stub.
    Daemon {
        /// Stay in the foreground (don't fork). Useful for systemd / development.
        #[arg(long)]
        foreground: bool,
    },
    /// Print or apply the Claude Code hook snippet for ~/.claude/settings.json. T080 implements.
    Install {
        /// Apply the snippet directly (writes to ~/.claude/settings.json). Default: print only.
        #[arg(long)]
        apply: bool,
    },
    /// Tail or query the audit log.
    Log {
        /// Show only denied decisions.
        #[arg(long, conflicts_with_all = ["allowed", "asked"])]
        denied: bool,
        /// Show only allowed decisions.
        #[arg(long, conflicts_with_all = ["denied", "asked"])]
        allowed: bool,
        /// Show only ask-decisions (i.e. ones that surfaced to a human).
        #[arg(long, conflicts_with_all = ["denied", "allowed"])]
        asked: bool,
        /// Only decisions newer than this (e.g. `1h`, `24h`, `7d`, `30m`).
        #[arg(long)]
        since: Option<String>,
        /// Only decisions older than this.
        #[arg(long)]
        until: Option<String>,
        /// Filter to one Claude Code session id.
        #[arg(long)]
        session: Option<String>,
        /// Filter to one tool name (Bash, Read, WebFetch, mcp__*, ...).
        #[arg(long)]
        tool: Option<String>,
        /// FTS5 search across tool_input + tool_name + cwd.
        #[arg(long)]
        grep: Option<String>,
        /// Maximum rows. Default 100.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Output newline-delimited JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// Order oldest-first instead of newest-first.
        #[arg(long)]
        reverse: bool,
    },
    /// Manage policy rules. T024 / T067 implement.
    Rule {
        #[command(subcommand)]
        action: Option<RuleAction>,
    },
    /// Manage learning suggestions. T067 implements.
    Learning {
        #[command(subcommand)]
        action: Option<LearningAction>,
    },
    /// Run a child command (typically `claude`) under the PTY-tap wrapper. T056 implements.
    Run {
        /// The command + args to spawn.
        #[arg(trailing_var_arg = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Start the MCP server on stdio or HTTP. T078 implements.
    Mcp {
        #[command(subcommand)]
        transport: Option<McpTransport>,
        /// Path to an agidb brain directory to serve `recall`/`timeline` against. Needs
        /// `--features brain-agidb`. Without it, the recall/timeline tools return "no brain".
        #[arg(long)]
        brain: Option<PathBuf>,
    },
    /// Invoked by Claude Code hooks via ~/.claude/settings.json. T029 implements.
    Hook {
        /// The hook event name (permission-request, notification, session-start, etc.).
        event: String,
    },
    /// One-command first-run: seed a policy, install the Claude Code hook, start the service.
    Setup {
        /// Set up the policy + hook but do not install a background service.
        #[arg(long)]
        no_service: bool,
        /// Which bundled policy to seed when none exists: default | strict | relaxed.
        #[arg(long)]
        policy: Option<String>,
    },
    /// Reverse `homn setup`: remove the service + hook. Keeps your policy + audit log.
    Uninstall {
        /// Also delete ~/.config/homn and ~/.local/share/homn (your policy + audit DB).
        #[arg(long)]
        purge: bool,
    },
    /// Verify the redaction/receipt hash chain (v2: US3 / FR-015).
    Ledger {
        /// Which ledger op to run.
        #[command(subcommand)]
        action: LedgerAction,
    },
    /// Phase-0 recall evaluation (v2: US1).
    Eval {
        /// Which eval op to run.
        #[command(subcommand)]
        action: EvalAction,
    },
    /// Add/list/remove a capture-exclude rule in `policies/ingest.rhai` (v2: US3 / T042).
    /// Hot-reloaded by a running daemon.
    Exclude {
        /// The app glob (e.g. `Slack*`) or domain (e.g. `github.com`) to exclude. Omit with `--list`.
        target: Option<String>,
        /// Treat <target> as a domain (exact match) instead of an app glob.
        #[arg(long)]
        domain: bool,
        /// List current `homn exclude` rules instead of adding one.
        #[arg(long)]
        list: bool,
        /// Remove the `homn exclude` rule matching <target>.
        #[arg(long)]
        remove: bool,
        /// Emit JSON (`{ "excludes": [{kind,target}] }`) for `--list`; machine-readable confirm otherwise.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum LedgerAction {
    /// Walk the hash-chained receipt ledger and report the first broken row, if any.
    Verify {
        /// Output JSON (`{"total","first_bad_seq","valid"}`) instead of human text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum EvalAction {
    /// Validate a question-set TOML file (counts, ids, expected refs) without running it.
    Validate {
        /// Path to the question-set TOML.
        file: PathBuf,
    },
    /// Score recall@k over an ingested brain. Needs `--features brain-agidb` + a `--brain` path.
    Run {
        /// Path to the question-set TOML.
        file: PathBuf,
        /// k for recall@k. Default 3 (the Phase-0 gate).
        #[arg(long, default_value_t = 3)]
        k: u32,
        /// Path to the agidb brain directory populated by `homn eval ingest`.
        #[arg(long)]
        brain: PathBuf,
        /// Emit JSON (`{ recall_at_1, recall_at_k, per_kind, gate }`) instead of human text.
        #[arg(long)]
        json: bool,
    },
    /// Throwaway Phase-0 replay-ingest: Screenpipe sqlite → agidb brain (no redaction, own data).
    /// Needs `--features brain-agidb`.
    Ingest {
        /// Path to the Screenpipe capture sqlite DB.
        screenpipe_db: PathBuf,
        /// Path to the agidb brain directory to populate (created if absent).
        #[arg(long)]
        brain: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum RuleAction {
    /// List rules across all policy files.
    List,
    /// Edit policies/default.rhai in $EDITOR.
    Edit,
    /// Append a rule to policies/default.rhai.
    Add { rule: String },
    /// Trace which rules match a given tool+input.
    Trace { tool: String, input: String },
}

#[derive(Subcommand, Debug)]
enum LearningAction {
    /// List open suggestions.
    List,
    /// Accept a suggestion by id; appends the rule to the appropriate policy file.
    Accept { id: i64 },
    /// Reject a suggestion (silenced for 30 days).
    Reject { id: i64 },
    /// Snooze a suggestion for a custom duration.
    Snooze { id: i64, days: u32 },
}

#[derive(Subcommand, Debug)]
enum McpTransport {
    /// stdio transport — the default for Claude Code MCP config.
    Stdio,
    /// Streamable HTTP transport.
    Http {
        #[arg(long, default_value = "127.0.0.1:9874")]
        bind: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Daemon { foreground }) => {
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path)?;
            tracing::info!(
                foreground,
                socket = %config.daemon.socket_path.display(),
                "starting daemon"
            );
            homn_daemon::run(config).await?;
        }
        Some(Command::Log {
            denied,
            allowed,
            asked,
            since,
            until,
            session,
            tool,
            grep,
            limit,
            json,
            reverse,
        }) => {
            log_command(LogArgs {
                denied,
                allowed,
                asked,
                since,
                until,
                session,
                tool,
                grep,
                limit,
                json,
                reverse,
            })
            .await?;
        }
        Some(Command::Install { apply }) => {
            let settings_path = homn_hook::default_settings_path();
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let report = homn_hook::run_install(&settings_path, apply, &mut lock)?;
            match report {
                homn_hook::InstallReport::Printed => {
                    // Output already written by run_install.
                }
                homn_hook::InstallReport::CreatedNew { path } => {
                    eprintln!(
                        "\nwrote new ~/.claude/settings.json with homn PermissionRequest hook"
                    );
                    eprintln!("path: {}", path.display());
                    eprintln!("next: start the daemon with `homn daemon` (and consider a systemd user unit)");
                }
                homn_hook::InstallReport::MergedExisting { path, backup } => {
                    eprintln!("\nmerged homn PermissionRequest hook into existing settings.json");
                    eprintln!("path:   {}", path.display());
                    eprintln!("backup: {}", backup.display());
                    eprintln!("next: start the daemon with `homn daemon`");
                }
                homn_hook::InstallReport::AlreadyPresent { path } => {
                    eprintln!(
                        "\nhomn hook is already installed in {}; nothing to do",
                        path.display()
                    );
                }
            }
        }
        Some(Command::Run { command }) => {
            // US3 slice B: PTY wrapper with deny enforcement (T054/T055).
            if command.is_empty() {
                anyhow::bail!("homn run requires a command to spawn (e.g. `homn run claude`)");
            }
            let config_path = homn_daemon::config::default_config_path();
            let daemon_config = homn_daemon::load_config(&config_path).unwrap_or_default();
            let prompt_regex = regex::Regex::new(&daemon_config.pty_wrapper.prompt_regex)
                .map_err(|e| anyhow::anyhow!("invalid pty_wrapper.prompt_regex: {e}"))?;
            let pty_config = homn_hook::PtyConfig {
                prompt_regex,
                audit_path: daemon_config.audit.db_path.clone(),
                deny_lookback_secs: 5,
                gating_enabled: daemon_config.pty_wrapper.enabled,
            };
            let result =
                tokio::task::spawn_blocking(move || homn_hook::run_under_pty(&command, pty_config))
                    .await??;
            std::process::exit(result.code);
        }
        Some(Command::Learning { action }) => {
            // US4 — learning subsystem CLI (T067).
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path).unwrap_or_default();
            if let Some(parent) = config.learning.db_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let db = homn_learning::Db::open(&config.learning.db_path).await?;
            match action.unwrap_or(LearningAction::List) {
                LearningAction::List => {
                    let suggestions = db.list_open().await?;
                    if suggestions.is_empty() {
                        eprintln!("(no open suggestions yet — use homn for a while)");
                    } else {
                        for s in &suggestions {
                            println!(
                                "#{id}  {verb:<5}  {repr}\n     observations: {count}  proposed rule:\n     {rule}\n",
                                id = s.id,
                                verb = s.proposed_verb,
                                repr = s.pattern_repr,
                                count = s.observation_count,
                                rule = s.proposed_rule,
                            );
                        }
                    }
                }
                LearningAction::Accept { id } => {
                    let suggestion = db.accept(id).await?;
                    let policy_file = config.policy.policies_dir.join(&suggestion.proposed_file);
                    let appended = homn_learning::append_rule_to_policy(&policy_file, &suggestion)?;
                    if appended {
                        eprintln!(
                            "appended rule to {}:\n  {}",
                            policy_file.display(),
                            suggestion.proposed_rule
                        );
                        eprintln!("(daemon will hot-reload within a few hundred ms)");
                    } else {
                        eprintln!(
                            "rule was already in {}; suggestion marked accepted",
                            policy_file.display()
                        );
                    }
                }
                LearningAction::Reject { id } => {
                    db.reject(id, 30).await?;
                    eprintln!("suggestion #{id} rejected; silenced for 30 days");
                }
                LearningAction::Snooze { id, days } => {
                    db.reject(id, days).await?;
                    eprintln!("suggestion #{id} snoozed for {days} days");
                }
            }
        }
        Some(Command::Hook { event }) => {
            // T029: read Claude Code hook payload from stdin, call the daemon, write the
            // expected hook-return JSON to stdout. Exit 0 ALWAYS so Claude falls back to its
            // own prompt rather than failing the request — see contracts/hook-protocol.md.
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path).unwrap_or_default();
            let mut buf = String::new();
            use tokio::io::AsyncReadExt;
            let _ = tokio::io::stdin().read_to_string(&mut buf).await;
            let response = match event.as_str() {
                "permission-request" => {
                    homn_hook::handle_permission_request(&config.daemon.socket_path, &buf).await
                }
                other => {
                    tracing::warn!(
                        event = other,
                        "hook event not yet handled; emitting empty response"
                    );
                    serde_json::json!({})
                }
            };
            println!("{}", serde_json::to_string(&response)?);
        }
        Some(Command::Mcp { transport, brain }) => {
            // T078: start the MCP server. stdio is what Claude Code's MCP config invokes;
            // HTTP transport (T071) lands in a follow-up.
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path).unwrap_or_default();
            let engine = homn_policy::Engine::new();
            let default_policy = config.policy.policies_dir.join("default.rhai");
            let rules = if default_policy.exists() {
                homn_policy::load_ruleset(&default_policy)?
            } else {
                homn_policy::RuleSet::parse(&engine, "", "default.rhai")?
            };
            let rules_handle: homn_policy::RuleSetHandle =
                std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(rules));
            if let Some(parent) = config.audit.db_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let audit = std::sync::Arc::new(homn_audit::Db::open(&config.audit.db_path).await?);
            let brain = open_brain(brain.as_deref()).await?;
            let state = homn_mcp::McpState {
                engine,
                rules: rules_handle,
                audit,
                brain,
            };
            match transport {
                Some(McpTransport::Stdio) | None => {
                    homn_mcp::serve_stdio(state).await?;
                }
                Some(McpTransport::Http { bind }) => {
                    anyhow::bail!(
                        "MCP HTTP transport is not implemented yet (would bind on {bind}); use `homn mcp stdio` for now"
                    );
                }
            }
        }
        Some(Command::Rule { action }) => {
            // T084 + rule CLI: list / edit / add / trace policy rules.
            rule_command(action.unwrap_or(RuleAction::List)).await?;
        }
        Some(Command::Setup { no_service, policy }) => {
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path).unwrap_or_default();
            let profile = match policy.as_deref() {
                Some("strict") => homn_hook::setup::PolicyProfile::Strict,
                Some("relaxed") => homn_hook::setup::PolicyProfile::Relaxed,
                Some("default") | None => homn_hook::setup::PolicyProfile::Default,
                Some(other) => {
                    anyhow::bail!("unknown --policy `{other}` (expected default|strict|relaxed)")
                }
            };
            let report = homn_hook::setup::run_setup(homn_hook::setup::SetupOptions {
                policies_dir: config.policy.policies_dir.clone(),
                settings_path: homn_hook::default_settings_path(),
                profile,
                install_service: !no_service,
            })?;
            print_setup_report(&report);
        }
        Some(Command::Uninstall { purge }) => {
            let config_path = homn_daemon::config::default_config_path();
            let config = homn_daemon::load_config(&config_path).unwrap_or_default();
            let report =
                homn_hook::setup::run_uninstall(&homn_hook::default_settings_path(), true)?;
            eprintln!(
                "hook removed: {}   service removed: {}",
                report.hook_removed, report.service_removed
            );
            if purge {
                if let Some(homn_cfg) = config.policy.policies_dir.parent() {
                    let _ = std::fs::remove_dir_all(homn_cfg);
                    eprintln!("purged {}", homn_cfg.display());
                }
                if let Some(audit_dir) = config.audit.db_path.parent() {
                    let _ = std::fs::remove_dir_all(audit_dir);
                    eprintln!("purged {}", audit_dir.display());
                }
            } else {
                eprintln!(
                    "kept your policy ({}) and audit log ({}) — use --purge to remove them",
                    config.policy.policies_dir.display(),
                    config.audit.db_path.display(),
                );
            }
        }
        Some(Command::Ledger { action }) => {
            ledger_command(action).await?;
        }
        Some(Command::Eval { action }) => {
            eval_command(action).await?;
        }
        Some(Command::Exclude {
            target,
            domain,
            list,
            remove,
            json,
        }) => {
            exclude_command(target, domain, list, remove, json).await?;
        }
        None => {
            // No subcommand → print short banner and help hint.
            println!(
                "homn {} — the homunculus for your coding agents",
                env!("CARGO_PKG_VERSION"),
            );
            println!("run `homn --help` to see available subcommands.");
        }
    }

    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

// ===== `homn log` =====================================================================

struct LogArgs {
    denied: bool,
    allowed: bool,
    asked: bool,
    since: Option<String>,
    until: Option<String>,
    session: Option<String>,
    tool: Option<String>,
    grep: Option<String>,
    limit: u32,
    json: bool,
    reverse: bool,
}

async fn log_command(args: LogArgs) -> anyhow::Result<()> {
    let config_path = homn_daemon::config::default_config_path();
    let config = homn_daemon::load_config(&config_path).unwrap_or_default();
    let db = homn_audit::Db::open(&config.audit.db_path).await?;

    let now_millis = chrono::Utc::now().timestamp_millis();
    let since_millis = args
        .since
        .as_deref()
        .map(parse_duration_to_past_millis)
        .transpose()?;
    let until_millis = args
        .until
        .as_deref()
        .map(parse_duration_to_past_millis)
        .transpose()?;

    let decision = if args.denied {
        Some(homn_types::Decision::Deny)
    } else if args.allowed {
        Some(homn_types::Decision::Allow)
    } else {
        None
    };

    let query = homn_audit::LogQuery {
        since_millis,
        until_millis,
        decision,
        asked: args.asked,
        session_id: args.session.clone(),
        tool_name: args.tool.clone(),
        grep: args.grep.clone(),
        limit: args.limit,
        ascending: args.reverse,
    };

    let rows = db.query(query).await?;

    if args.json {
        for row in &rows {
            println!("{}", serde_json::to_string(row)?);
        }
    } else {
        let _ = now_millis; // (reserved for "X seconds ago" rendering)
        let tty = is_terminal::IsTerminal::is_terminal(&std::io::stdout());
        let style = if tty { Style::ansi() } else { Style::plain() };
        for row in &rows {
            render_row_human(row, &style);
        }
        if rows.is_empty() {
            eprintln!("(no matching decisions)");
        }
    }
    Ok(())
}

/// Parse `"1h"`, `"30m"`, `"7d"` → unix-epoch-millis at `now - that duration`.
fn parse_duration_to_past_millis(s: &str) -> anyhow::Result<i64> {
    let dur =
        humantime::parse_duration(s).map_err(|e| anyhow::anyhow!("invalid duration `{s}`: {e}"))?;
    let now = chrono::Utc::now();
    let past = now - chrono::Duration::from_std(dur).unwrap_or_else(|_| chrono::Duration::zero());
    Ok(past.timestamp_millis())
}

struct Style {
    red: &'static str,
    green: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    dim: &'static str,
    reset: &'static str,
}

impl Style {
    fn ansi() -> Self {
        Self {
            red: "\x1b[31m",
            green: "\x1b[32m",
            yellow: "\x1b[33m",
            cyan: "\x1b[36m",
            dim: "\x1b[2m",
            reset: "\x1b[0m",
        }
    }
    fn plain() -> Self {
        Self {
            red: "",
            green: "",
            yellow: "",
            cyan: "",
            dim: "",
            reset: "",
        }
    }
}

fn render_row_human(row: &homn_types::DecisionRecord, s: &Style) {
    let ts = chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(row.ts_millis as u64),
    )
    .format("%Y-%m-%d %H:%M:%S");

    let (color, decision_str) = match row.decision {
        homn_types::Decision::Allow => (s.green, "allow"),
        homn_types::Decision::Deny => (s.red, "deny"),
        homn_types::Decision::Ask => (s.yellow, "ask"),
    };

    // Header line: timestamp + decision + tool + preview
    let preview = preview_tool_input(&row.tool_input);
    println!(
        "{dim}{ts}{reset}  {color}{decision:<5}{reset}  {cyan}{tool:<10}{reset}  {preview}",
        dim = s.dim,
        reset = s.reset,
        color = color,
        decision = decision_str,
        cyan = s.cyan,
        tool = row.tool_name,
        preview = preview,
    );

    // Optional sub-line: rule that fired.
    if let Some(loc) = &row.rule_source {
        let line = loc.line;
        let file = loc.file.display();
        let rule_text = row.rule_text.as_deref().unwrap_or("").trim();
        println!(
            "                       {dim}rule: {file}:{line}{reset}  {dim}— {rule_text}{reset}",
            dim = s.dim,
            reset = s.reset,
        );
    }

    // Sub-line: session, cwd, latency, source.
    println!(
        "                       {dim}session: {session}  cwd: {cwd}  latency: {latency}ms  via: {source:?}{reset}",
        dim = s.dim,
        reset = s.reset,
        session = row.session_id,
        cwd = row.cwd.display(),
        latency = row.latency_ms,
        source = row.source,
    );
}

fn preview_tool_input(v: &serde_json::Value) -> String {
    // For common tools, surface the most-readable field. Fall back to the whole JSON.
    if let Some(cmd) = v.get("command").and_then(|x| x.as_str()) {
        return clip(cmd);
    }
    if let Some(path) = v.get("path").and_then(|x| x.as_str()) {
        return clip(path);
    }
    if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
        return clip(url);
    }
    clip(&v.to_string())
}

fn clip(s: &str) -> String {
    if s.len() <= 120 {
        s.to_owned()
    } else {
        format!("{}…", &s[..119])
    }
}

// ===== `homn rule` ====================================================================

/// Baked-in copy of the canonical starter policy. `homn rule edit` writes this when no
/// policy file exists yet, so a fresh install is never staring at an empty file.
const STARTER_POLICY: &str = include_str!("../../../policies/default.rhai");

async fn rule_command(action: RuleAction) -> anyhow::Result<()> {
    let config_path = homn_daemon::config::default_config_path();
    let config = homn_daemon::load_config(&config_path).unwrap_or_default();
    let configured = config.policy.policies_dir.join("default.rhai");

    match action {
        RuleAction::List => {
            let path = resolve_readable_policy(&configured)?;
            let engine = homn_policy::Engine::new();
            let rules = homn_policy::RuleSet::load(&engine, &path)
                .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", path.display()))?;
            print_rule_list(&path, &rules);
        }
        RuleAction::Trace { tool, input } => {
            let path = resolve_readable_policy(&configured)?;
            let engine = homn_policy::Engine::new();
            let rules = homn_policy::RuleSet::load(&engine, &path)
                .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", path.display()))?;
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let tool_input = tool_input_from_str(&tool, &input);
            let req = homn_policy::EvalRequest::from_tool_call(&tool, &tool_input, &cwd);
            print_trace(&path, &tool, &input, &engine.trace(&rules, &req));
        }
        RuleAction::Add { rule } => {
            // Validate before touching the file — a rule that doesn't parse never lands.
            let engine = homn_policy::Engine::new();
            homn_policy::RuleSet::parse(&engine, &rule, "default.rhai")
                .map_err(|e| anyhow::anyhow!("refusing to add — rule does not parse: {e}"))?;
            if let Some(parent) = configured.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut text = std::fs::read_to_string(&configured).unwrap_or_default();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            let today = chrono::Local::now().format("%Y-%m-%d");
            text.push_str(&format!(
                "\n// added via `homn rule add` on {today}\n{rule}\n"
            ));
            std::fs::write(&configured, text)?;
            eprintln!("appended to {}:", configured.display());
            eprintln!("  {rule}");
            eprintln!("(a running daemon hot-reloads the change within ~50ms)");
        }
        RuleAction::Edit => {
            if !configured.exists() {
                if let Some(parent) = configured.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&configured, STARTER_POLICY)?;
                eprintln!("seeded {} with the starter policy", configured.display());
            }
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let status = std::process::Command::new(&editor)
                .arg(&configured)
                .status()
                .map_err(|e| anyhow::anyhow!("failed to launch $EDITOR ({editor}): {e}"))?;
            if !status.success() {
                anyhow::bail!("$EDITOR ({editor}) exited with a failure status");
            }
        }
    }
    Ok(())
}

/// Resolve the policy file to read: the configured path, else a repo-local sample (handy
/// when running from a checkout), else a helpful error.
fn resolve_readable_policy(configured: &Path) -> anyhow::Result<PathBuf> {
    if configured.exists() {
        return Ok(configured.to_path_buf());
    }
    let repo_local = PathBuf::from("policies/default.rhai");
    if repo_local.exists() {
        return Ok(repo_local);
    }
    anyhow::bail!(
        "no policy file found\n  expected: {}\n  fix:      run `homn rule edit` to create one",
        configured.display(),
    )
}

/// Build a `tool_input` JSON object from a CLI string. Accepts explicit JSON, otherwise wraps
/// the string under the key the tool expects (`command` / `path` / `url`).
fn tool_input_from_str(tool: &str, input: &str) -> serde_json::Value {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
        if v.is_object() {
            return v;
        }
    }
    let key = match tool {
        "Read" | "Edit" | "Write" | "NotebookEdit" => "path",
        "WebFetch" | "WebSearch" => "url",
        _ => "command",
    };
    serde_json::json!({ key: input })
}

fn print_rule_list(path: &Path, rules: &homn_policy::RuleSet) {
    let tty = is_terminal::IsTerminal::is_terminal(&std::io::stdout());
    let s = if tty { Style::ansi() } else { Style::plain() };

    let deny: Vec<(u32, String)> = rules
        .deny_rules()
        .map(|r| (r.line(), r.source_text().to_owned()))
        .collect();
    let ask: Vec<(u32, String)> = rules
        .ask_rules()
        .map(|r| (r.line(), r.source_text().to_owned()))
        .collect();
    let allow: Vec<(u32, String)> = rules
        .allow_rules()
        .map(|r| (r.line(), r.source_text().to_owned()))
        .collect();

    println!();
    println!("  {}policy:{} {}", s.dim, s.reset, path.display());
    println!(
        "  {}{} deny · {} ask · {} allow{}",
        s.dim,
        deny.len(),
        ask.len(),
        allow.len(),
        s.reset,
    );
    print_verb_group("DENY", s.red, &s, &deny);
    print_verb_group("ASK", s.yellow, &s, &ask);
    print_verb_group("ALLOW", s.green, &s, &allow);
    println!();
}

fn print_verb_group(label: &str, color: &str, s: &Style, rules: &[(u32, String)]) {
    if rules.is_empty() {
        return;
    }
    println!();
    println!("  {}{}{}", color, label, s.reset);
    for (line, text) in rules {
        println!("    {}{:>3}{}  {}", s.dim, line, s.reset, text);
    }
}

fn print_trace(path: &Path, tool: &str, input: &str, trace: &homn_policy::Trace) {
    let tty = is_terminal::IsTerminal::is_terminal(&std::io::stdout());
    let s = if tty { Style::ansi() } else { Style::plain() };

    println!();
    println!(
        "  {}trace{}  {}{}{}: {}",
        s.cyan, s.reset, s.cyan, tool, s.reset, input,
    );
    println!("  {}policy: {}{}", s.dim, path.display(), s.reset);
    println!();

    for rt in &trace.rules {
        let (vcolor, verb) = verb_style(rt.verb, &s);
        let marker = if rt.matched {
            format!("{}● match {}", vcolor, s.reset)
        } else {
            format!("{}○       {}", s.dim, s.reset)
        };
        let decisive = if rt.decisive {
            format!("  {}← decides{}", vcolor, s.reset)
        } else {
            String::new()
        };
        // Dim rules that didn't fire so the eye lands on the ones that did.
        let (tp, tr) = if rt.matched {
            ("", "")
        } else {
            (s.dim, s.reset)
        };
        println!(
            "  {marker}  {vcolor}{verb:<5}{reset} {dim}{file}:{line}{reset}  {tp}{text}{tr}{decisive}",
            vcolor = vcolor,
            reset = s.reset,
            dim = s.dim,
            file = rt.location.file.display(),
            line = rt.location.line,
            text = rt.source_text,
        );
    }

    println!();
    let (dcolor, decision) = verb_style(trace.outcome.decision, &s);
    match &trace.outcome.rule {
        Some(loc) => println!(
            "  decision: {}{}{}  (matched {}:{})",
            dcolor,
            decision.to_uppercase(),
            s.reset,
            loc.file.display(),
            loc.line,
        ),
        None => println!(
            "  decision: {}{}{}  (no rule matched — default ask)",
            dcolor,
            decision.to_uppercase(),
            s.reset,
        ),
    }
    println!();
}

fn verb_style(d: homn_types::Decision, s: &Style) -> (&'static str, &'static str) {
    match d {
        homn_types::Decision::Deny => (s.red, "deny"),
        homn_types::Decision::Ask => (s.yellow, "ask"),
        homn_types::Decision::Allow => (s.green, "allow"),
    }
}

// ===== `homn setup` reporting ==========================================================

fn print_setup_report(report: &homn_hook::setup::SetupReport) {
    use homn_hook::setup::{PolicySeedOutcome, ServiceOutcome};

    eprintln!("\nhomn setup");
    match &report.policy {
        PolicySeedOutcome::Written(p) => eprintln!("  policy:   seeded {}", p.display()),
        PolicySeedOutcome::KeptExisting(p) => {
            eprintln!("  policy:   kept your existing {}", p.display())
        }
        PolicySeedOutcome::KeptUnparseable(p) => eprintln!(
            "  policy:   WARNING {} does not parse — left untouched; fix it with `homn rule edit`",
            p.display()
        ),
    }
    eprintln!("  hook:     installed into Claude Code settings.json");
    match &report.service {
        ServiceOutcome::Installed(p) => {
            eprintln!("  service:  installed + started ({})", p.display())
        }
        ServiceOutcome::SkippedByFlag => {
            eprintln!("  service:  skipped (--no-service) — run `homn daemon` yourself")
        }
        ServiceOutcome::UnsupportedPlatform => eprintln!(
            "  service:  unsupported platform — start `homn daemon` manually or via your init system"
        ),
    }
    eprintln!("\ndone. edit your rules anytime with `homn rule edit`.");
}

// ===== `homn ledger verify` (v2: US3 / FR-015) ==========================================

async fn ledger_command(action: LedgerAction) -> anyhow::Result<()> {
    let LedgerAction::Verify { json } = action;
    let config_path = homn_daemon::config::default_config_path();
    let config = homn_daemon::load_config(&config_path).unwrap_or_default();
    let db = homn_audit::Db::open(&config.audit.db_path).await?;
    let v = db.verify_ledger().await?;
    if json {
        let out = serde_json::json!({
            "total": v.total,
            "first_bad_seq": v.first_bad_seq,
            "valid": v.is_valid(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if v.is_valid() {
        println!("ledger OK — {} receipts, hash chain verified", v.total);
    } else {
        let bad = v.first_bad_seq.unwrap_or(-1);
        eprintln!(
            "ledger BROKEN — {} receipts, first untrusted row seq={bad}",
            v.total
        );
        std::process::exit(1);
    }
    Ok(())
}

// ===== `homn exclude` (v2: US3 / T042) ================================================
//
// Edits `policies/ingest.rhai` to add/list/remove a capture-exclude rule. Each rule is a
// marked two-line block so it can be found back without parsing Rhai:
//   // homn:exclude app Slack*
//   if app.matches("Slack*") { deny_with("exclude.cli"); }
// A running daemon hot-reloads the file (FR-013).

async fn exclude_command(
    target: Option<String>,
    domain: bool,
    list: bool,
    remove: bool,
    json: bool,
) -> anyhow::Result<()> {
    let config_path = homn_daemon::config::default_config_path();
    let config = homn_daemon::load_config(&config_path)?;
    let path = config.policy.policies_dir.join("ingest.rhai");

    if list && remove {
        anyhow::bail!("--list and --remove are mutually exclusive");
    }

    if list {
        let excludes = list_excludes(&path)?;
        if json {
            let arr: Vec<_> = excludes
                .iter()
                .map(|(kind, t)| serde_json::json!({"kind": kind, "target": t}))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"excludes": arr}))?
            );
        } else {
            if excludes.is_empty() {
                println!("(no `homn exclude` rules in {})", path.display());
            } else {
                for (kind, t) in &excludes {
                    println!("{kind}\t{t}");
                }
            }
        }
        return Ok(());
    }

    let target = target.ok_or_else(|| {
        anyhow::anyhow!("a target is required (e.g. `homn exclude Slack*` or `homn exclude github.com --domain`)")
    })?;

    if remove {
        let removed = remove_exclude(&path, &target, domain)?;
        if removed {
            eprintln!(
                "removed exclude `{target}` from {} (a running daemon hot-reloads within ~50ms)",
                path.display()
            );
        } else {
            eprintln!(
                "no `homn exclude` rule matching `{target}` found in {}",
                path.display()
            );
        }
        return Ok(());
    }

    // Add.
    let added = add_exclude(&path, &target, domain)?;
    if added {
        eprintln!(
            "added exclude `{target}` ({}) to {}",
            exclude_kind(domain),
            path.display()
        );
        eprintln!("(a running daemon hot-reloads the change within ~50ms)");
    } else {
        eprintln!("exclude `{target}` already present in {}", path.display());
    }
    Ok(())
}

/// Add a marked exclude block to `path`. Returns `false` if an identical exclude already exists.
fn add_exclude(path: &Path, target: &str, domain: bool) -> anyhow::Result<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if list_excludes(path)?
        .iter()
        .any(|(k, t)| *k == exclude_kind(domain) && t == target)
    {
        return Ok(false);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let today = chrono::Local::now().format("%Y-%m-%d");
    let kind = exclude_kind(domain);
    text.push_str(&format!(
        "\n// homn:exclude {kind} {target}  (added via `homn exclude` on {today})\n{rule}\n",
        rule = exclude_rule(target, domain)
    ));
    std::fs::write(path, text)?;
    Ok(true)
}

fn exclude_kind(domain: bool) -> &'static str {
    if domain {
        "domain"
    } else {
        "app"
    }
}

fn exclude_rule(target: &str, domain: bool) -> String {
    if domain {
        format!("if domain == \"{target}\" {{ deny_with(\"exclude.cli\"); }}")
    } else {
        format!("if app.matches(\"{target}\") {{ deny_with(\"exclude.cli\"); }}")
    }
}

/// Parse the marked `// homn:exclude <kind> <target>` lines out of the ingest policy.
fn list_excludes(path: &Path) -> anyhow::Result<Vec<(&'static str, String)>> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// homn:exclude ") {
            let mut parts = rest.split_whitespace();
            let kind = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            if target.is_empty() {
                continue;
            }
            let kind_str: &'static str = match kind {
                "app" => "app",
                "domain" => "domain",
                _ => "app",
            };
            out.push((kind_str, target.to_owned()));
        }
    }
    Ok(out)
}

/// Remove the marked two-line block whose target matches.
fn remove_exclude(path: &Path, target: &str, domain: bool) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path)?;
    let marker = format!("// homn:exclude {} {}", exclude_kind(domain), target);
    let mut lines: Vec<String> = text.lines().map(|l| l.to_owned()).collect();
    let mut removed = false;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with(&marker) {
            // Remove the marker line + the following rule line (if present).
            lines.remove(i);
            if i < lines.len() && lines[i].trim_start().starts_with("if ") {
                lines.remove(i);
            }
            // Collapse a single trailing blank line left behind.
            if i > 0
                && i < lines.len()
                && lines[i - 1].trim().is_empty()
                && lines[i].trim().is_empty()
            {
                lines.remove(i);
            }
            removed = true;
            break;
        }
        i += 1;
    }
    if !removed {
        return Ok(false);
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(removed)
}

/// Open an agidb brain for the MCP `recall`/`timeline` tools. `None` → no brain (the tools
/// return a clear "no brain" error). Needs `--features brain-agidb`; without the feature, any
/// `--brain` path is reported as needing a rebuild.
async fn open_brain(
    path: Option<&Path>,
) -> anyhow::Result<Option<std::sync::Arc<dyn homn_mcp::Brain>>> {
    let Some(path) = path else { return Ok(None) };
    #[cfg(not(feature = "brain-agidb"))]
    {
        anyhow::bail!(
            "`homn mcp --brain {}` needs the agidb brain. Rebuild with:\n  \
             cargo build -p homn-bin --features brain-agidb",
            path.display()
        );
    }
    #[cfg(feature = "brain-agidb")]
    {
        let brain = agidb::Agidb::open_with(
            agidb::AgidbConfig::new(path).with_extractor(agidb::ExtractorSetup::Null),
        )
        .await
        .map_err(|e| anyhow::anyhow!("open brain {}: {e}", path.display()))?;
        Ok(Some(std::sync::Arc::new(homn_mcp::AgidbBrain::new(
            std::sync::Arc::new(brain),
        ))))
    }
}

// ===== `homn eval` (v2: US1) ===========================================================

async fn eval_command(action: EvalAction) -> anyhow::Result<()> {
    match action {
        EvalAction::Validate { file } => {
            let src = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", file.display()))?;
            let set = homn_eval::QuestionSet::from_toml_str(&src)
                .map_err(|e| anyhow::anyhow!("parse {}: {e}", file.display()))?;
            set.validate(true)
                .map_err(|e| anyhow::anyhow!("invalid question set: {e}"))?;
            let counts = set.counts();
            let n = set.questions.len();
            println!(
                "OK — {n} questions: {:?}",
                counts
                    .into_iter()
                    .map(|(k, c)| (format!("{k:?}"), c))
                    .collect::<Vec<_>>()
            );
            Ok(())
        }
        EvalAction::Run {
            file,
            k,
            brain,
            json,
        } => {
            #[cfg(not(feature = "brain-agidb"))]
            {
                let _ = (file, k, brain, json);
                eprintln!(
                    "homn eval run needs the agidb brain. Rebuild with:\n  \
                     cargo build -p homn-bin --features brain-agidb\nthen pass --brain <path>."
                );
                std::process::exit(2);
            }
            #[cfg(feature = "brain-agidb")]
            {
                let src = std::fs::read_to_string(&file)
                    .map_err(|e| anyhow::anyhow!("read {}: {e}", file.display()))?;
                let set = homn_eval::QuestionSet::from_toml_str(&src)
                    .map_err(|e| anyhow::anyhow!("parse {}: {e}", file.display()))?;
                set.validate(true)
                    .map_err(|e| anyhow::anyhow!("invalid question set: {e}"))?;
                // Run the sync recaller + scorer on a blocking thread: the recaller owns a
                // private Tokio runtime which must both be created and dropped outside an
                // async context (block_on inside a runtime panics; dropping a runtime inside
                // one does too).
                let brain_path = brain.clone();
                let (result, branch) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                    let recaller = homn_eval::ingest::AgidbRecaller::open(&brain_path)
                        .map_err(|e| anyhow::anyhow!("open brain: {e}"))?;
                    let result = homn_eval::score(&set, &recaller, k as usize);
                    let branch = homn_eval::gate_verdict(result.recall_at_k);
                    Ok((result, branch))
                })
                .await??;
                if json {
                    let json_out = serde_json::json!({
                        "k": result.k,
                        "total": result.total,
                        "recall_at_1": result.recall_at_1,
                        "recall_at_k": result.recall_at_k,
                        "per_kind_recall_at_k": result.per_kind_recall_at_k,
                        "ops": result.ops,
                        "gate": branch,
                    });
                    println!("{}", serde_json::to_string_pretty(&json_out)?);
                } else {
                    print!("{}", homn_eval::format_report(&result, branch));
                }
                Ok(())
            }
        }
        EvalAction::Ingest {
            screenpipe_db,
            brain,
        } => {
            #[cfg(not(feature = "brain-agidb"))]
            {
                let _ = (screenpipe_db, brain);
                eprintln!(
                    "homn eval ingest needs the agidb brain. Rebuild with:\n  \
                     cargo build -p homn-bin --features brain-agidb"
                );
                std::process::exit(2);
            }
            #[cfg(feature = "brain-agidb")]
            {
                use homn_eval::ingest::{replay_ingest, IngestConfig};

                if !screenpipe_db.exists() {
                    anyhow::bail!("screenpipe db not found: {}", screenpipe_db.display());
                }
                let agidb_cfg =
                    agidb::AgidbConfig::new(&brain).with_extractor(agidb::ExtractorSetup::Null);
                let agidb_brain = agidb::Agidb::open_with(agidb_cfg)
                    .await
                    .map_err(|e| anyhow::anyhow!("open brain {}: {e}", brain.display()))?;
                let report =
                    replay_ingest(&screenpipe_db, &agidb_brain, &IngestConfig::default()).await?;
                agidb_brain
                    .flush()
                    .await
                    .map_err(|e| anyhow::anyhow!("flush brain: {e}"))?;
                println!(
                    "ingested: {} rows read, {} chunks stored into {}",
                    report.rows_read,
                    report.chunks_stored,
                    brain.display()
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod exclude_tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_ingest(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("homn-exclude-{}-{name}.rhai", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn add_list_remove_roundtrip_app() {
        let p = tmp_ingest("app");
        // Add an app exclude.
        assert!(
            add_exclude(&p, "Slack*", false).unwrap(),
            "first add inserts"
        );
        // Duplicate is a no-op.
        assert!(
            !add_exclude(&p, "Slack*", false).unwrap(),
            "duplicate not re-added"
        );
        // Listed.
        let listed = list_excludes(&p).unwrap();
        assert_eq!(listed, vec![("app", "Slack*".to_owned())]);
        // The rule text is present and parses-shaped.
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("if app.matches(\"Slack*\")"), "{text}");
        // Remove.
        assert!(
            remove_exclude(&p, "Slack*", false).unwrap(),
            "remove finds it"
        );
        assert!(list_excludes(&p).unwrap().is_empty(), "gone after remove");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn add_list_remove_domain() {
        let p = tmp_ingest("domain");
        assert!(add_exclude(&p, "github.com", true).unwrap());
        let listed = list_excludes(&p).unwrap();
        assert_eq!(listed, vec![("domain", "github.com".to_owned())]);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("if domain == \"github.com\""), "{text}");
        assert!(remove_exclude(&p, "github.com", true).unwrap());
        assert!(list_excludes(&p).unwrap().is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn remove_missing_is_false_not_error() {
        let p = tmp_ingest("missing");
        std::fs::write(&p, "// unrelated\nfn x() {}\n").unwrap();
        assert!(!remove_exclude(&p, "nope", false).unwrap());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn add_preserves_existing_policy_text() {
        let p = tmp_ingest("preserve");
        std::fs::write(
            &p,
            "// my policy\nif incognito { deny_with(\"browser.incognito\"); }\n",
        )
        .unwrap();
        assert!(add_exclude(&p, "1Password*", false).unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("browser.incognito"),
            "existing rule kept: {text}"
        );
        assert!(text.contains("1Password*"), "new exclude appended: {text}");
        let _ = std::fs::remove_file(&p);
    }
}
