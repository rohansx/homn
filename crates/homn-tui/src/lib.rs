//! TUI surface for `homn` permission prompts (T031).
//!
//! When a policy decision is `Ask`, the hook subprocess opens an interactive prompt on
//! `/dev/tty` directly (not stdin/stdout, which Claude Code has plumbed for the JSON
//! hook protocol). The user picks one of `[a]llow`, `[d]eny`, `[A]lways-allow`,
//! `[D]always-deny`, or presses Enter / `q` to defer back to Claude's own prompt.
//!
//! See [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md)
//! §"TUI prompt" for the long-form rationale.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use homn_types::{HumanAnswer, RuleSourceLocation};
use serde::{Deserialize, Serialize};

/// Everything the prompt needs to render. Built by the hook from the daemon's
/// `decisions.create` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPayload {
    /// Audit-log id; the hook echoes this back when the user answers via `decisions.resolve`.
    pub decision_id: i64,
    /// The session that triggered this decision.
    pub session_id: String,
    /// Tool name (e.g. `"Bash"`).
    pub tool_name: String,
    /// A short, human-readable summary of the tool input (e.g. the command, the path, the URL).
    pub tool_input_preview: String,
    /// Calling session's working directory.
    pub cwd: PathBuf,
    /// The rule that matched, if any.
    pub rule_source: Option<RuleSourceLocation>,
    /// Snapshot of the matching rule's source text.
    pub rule_text: Option<String>,
}

/// Result of asking the user. `None` means the user deferred (`q` / Enter / no `/dev/tty`).
pub fn prompt_user(payload: &PromptPayload) -> Option<HumanAnswer> {
    // The hook subprocess has stdin/stdout reserved for the JSON hook protocol. We open
    // /dev/tty directly so we can render to the user's terminal regardless of pipes.
    let tty_write = match File::options().write(true).open("/dev/tty") {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(error = %err, "no /dev/tty available; deferring to claude");
            return None;
        }
    };

    let tty_read = match File::options().read(true).open("/dev/tty") {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(error = %err, "couldn't open /dev/tty for reading");
            return None;
        }
    };

    render_and_read(payload, tty_write, tty_read)
}

fn render_and_read<R, W>(payload: &PromptPayload, mut writer: W, reader: R) -> Option<HumanAnswer>
where
    W: Write,
    R: std::io::Read,
{
    let tty = is_terminal::IsTerminal::is_terminal(&std::io::stderr());
    let style = if tty { Style::ansi() } else { Style::plain() };
    let _ = render_prompt(payload, &mut writer, &style);
    let _ = writer.flush();

    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    match buf.read_line(&mut line) {
        Ok(0) | Err(_) => {
            tracing::warn!("TTY closed before user answered; deferring");
            None
        }
        Ok(_) => parse_answer(line.trim()),
    }
}

fn render_prompt<W: Write>(
    payload: &PromptPayload,
    out: &mut W,
    style: &Style,
) -> std::io::Result<()> {
    let header = format!(
        "{bold}═══ homn: permission request ═══{reset}",
        bold = style.bold,
        reset = style.reset,
    );
    writeln!(out)?;
    writeln!(out, "{header}")?;
    writeln!(
        out,
        "  session: {}{}{}",
        style.dim, payload.session_id, style.reset
    )?;
    writeln!(
        out,
        "  tool:    {}{}{}",
        style.cyan, payload.tool_name, style.reset
    )?;
    if !payload.tool_input_preview.is_empty() {
        writeln!(
            out,
            "  input:   {}{}{}",
            style.bold, payload.tool_input_preview, style.reset
        )?;
    }
    writeln!(
        out,
        "  cwd:     {}{}{}",
        style.dim,
        payload.cwd.display(),
        style.reset
    )?;
    if let Some(loc) = &payload.rule_source {
        let txt = payload.rule_text.as_deref().unwrap_or("");
        writeln!(
            out,
            "  rule:    {dim}{file}:{line}{reset}  {dim}— {txt}{reset}",
            file = loc.file.display(),
            line = loc.line,
            dim = style.dim,
            reset = style.reset,
        )?;
    } else {
        writeln!(
            out,
            "  rule:    {dim}(no match — default ask){reset}",
            dim = style.dim,
            reset = style.reset,
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "  {green}[a]{reset}llow   {red}[d]{reset}eny   {green}[A]{reset}lways-allow   {red}[D]{reset}always-deny   (Enter to defer)",
        green = style.green,
        red = style.red,
        reset = style.reset,
    )?;
    write!(out, "  > ")?;
    Ok(())
}

fn parse_answer(input: &str) -> Option<HumanAnswer> {
    match input {
        "a" => Some(HumanAnswer::Allow),
        "d" => Some(HumanAnswer::Deny),
        "A" => Some(HumanAnswer::AlwaysAllow),
        "D" => Some(HumanAnswer::AlwaysDeny),
        // Anything else — including "q", empty Enter, "quit", or typos — means "defer".
        // We never silently force a decision the user didn't explicitly choose.
        _ => None,
    }
}

struct Style {
    bold: &'static str,
    dim: &'static str,
    cyan: &'static str,
    green: &'static str,
    red: &'static str,
    reset: &'static str,
}

impl Style {
    fn ansi() -> Self {
        Self {
            bold: "\x1b[1m",
            dim: "\x1b[2m",
            cyan: "\x1b[36m",
            green: "\x1b[32m",
            red: "\x1b[31m",
            reset: "\x1b[0m",
        }
    }
    fn plain() -> Self {
        Self {
            bold: "",
            dim: "",
            cyan: "",
            green: "",
            red: "",
            reset: "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn sample() -> PromptPayload {
        PromptPayload {
            decision_id: 42,
            session_id: "01HXY".into(),
            tool_name: "Bash".into(),
            tool_input_preview: "git push origin main".into(),
            cwd: PathBuf::from("/home/rsx/dev"),
            rule_source: Some(RuleSourceLocation {
                file: PathBuf::from("default.rhai"),
                line: 14,
            }),
            rule_text: Some("ask if cmd.matches(\"git push * main\")".into()),
        }
    }

    #[test]
    fn parse_answer_maps_known_letters() {
        assert_eq!(parse_answer("a"), Some(HumanAnswer::Allow));
        assert_eq!(parse_answer("d"), Some(HumanAnswer::Deny));
        assert_eq!(parse_answer("A"), Some(HumanAnswer::AlwaysAllow));
        assert_eq!(parse_answer("D"), Some(HumanAnswer::AlwaysDeny));
    }

    #[test]
    fn parse_answer_returns_none_for_defer_signals() {
        assert_eq!(parse_answer(""), None);
        assert_eq!(parse_answer("q"), None);
        assert_eq!(parse_answer("quit"), None);
        assert_eq!(parse_answer("garbage"), None);
        assert_eq!(
            parse_answer(" a "),
            None,
            "we expect a clean letter, not padded"
        );
    }

    #[test]
    fn render_prompt_includes_key_fields() {
        let mut out = Vec::new();
        let style = Style::plain();
        render_prompt(&sample(), &mut out, &style).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("Bash"));
        assert!(rendered.contains("git push origin main"));
        assert!(rendered.contains("default.rhai:14"));
        assert!(rendered.contains("homn: permission request"));
    }

    #[test]
    fn render_and_read_routes_user_choice() {
        let mut out = Vec::new();
        let input = Cursor::new(b"a\n".to_vec());
        let ans = render_and_read(&sample(), &mut out, input);
        assert_eq!(ans, Some(HumanAnswer::Allow));
    }

    #[test]
    fn render_and_read_returns_none_on_empty_input() {
        let mut out = Vec::new();
        let input: &[u8] = b"";
        let ans = render_and_read(&sample(), &mut out, input);
        assert_eq!(ans, None);
    }

    #[test]
    fn no_rule_source_renders_default_ask_marker() {
        let mut out = Vec::new();
        let mut p = sample();
        p.rule_source = None;
        p.rule_text = None;
        let style = Style::plain();
        render_prompt(&p, &mut out, &style).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("default ask"));
    }
}
