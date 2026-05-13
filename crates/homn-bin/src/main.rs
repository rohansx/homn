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
    /// Tail or query the audit log. T042 implements.
    Log {
        /// Show only denied decisions.
        #[arg(long)]
        denied: bool,
        /// Show only allowed decisions.
        #[arg(long)]
        allowed: bool,
        /// Show only ask-resolved decisions.
        #[arg(long)]
        asked: bool,
        /// Output as newline-delimited JSON.
        #[arg(long)]
        json: bool,
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
        .try_init();
}
