//! `homn` binary entry point.
//!
//! Subcommand dispatch via `clap` derive. Each subcommand is a thin wrapper that calls into one
//! of the lib crates (`homn-daemon`, `homn-hook`, `homn-tui`, etc.).
//!
//! T001 (this file): skeleton dispatcher with `--version` and `--help` working; subcommand stubs
//! that print a "not yet implemented in T0XX" message and exit non-zero.

#![forbid(unsafe_code)]

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
    },
    /// Invoked by Claude Code hooks via ~/.claude/settings.json. T029 implements.
    Hook {
        /// The hook event name (permission-request, notification, session-start, etc.).
        event: String,
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
        Some(other) => {
            anyhow::bail!(
                "subcommand `{other:?}` is not implemented yet — see specs/001-policy-engine/tasks.md"
            );
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
