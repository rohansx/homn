//! Parser for the homn rule DSL.
//!
//! Grammar (one rule per non-empty, non-comment line):
//!
//! ```text
//! <verb> if <expression>;
//! ```
//!
//! `<verb>` ∈ {`allow`, `deny`, `ask`}. `<expression>` is a Rhai boolean expression compiled once
//! and reused. Comments start with `//`. Lines that are blank or comment-only are skipped.

use std::path::{Path, PathBuf};

use rhai::AST;

use crate::Engine;

/// Errors that can occur while parsing a policy file.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Failed to read the file from disk.
    #[error("io error reading {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A rule line doesn't match the expected `<verb> if <expr>;` shape.
    #[error("{file}:{line}: malformed rule: {message}")]
    Malformed {
        /// File name (display only).
        file: String,
        /// 1-indexed line number.
        line: u32,
        /// What went wrong.
        message: String,
    },
    /// The expression after `if` doesn't compile as Rhai.
    #[error("{file}:{line}: rhai compile error: {message}")]
    BadExpression {
        /// File name (display only).
        file: String,
        /// 1-indexed line number.
        line: u32,
        /// The error from Rhai.
        message: String,
    },
}

/// One compiled rule.
#[derive(Debug)]
pub struct CompiledRule {
    verb: Verb,
    file_name: String,
    line: u32,
    source_text: String,
    ast: AST,
}

impl CompiledRule {
    /// Borrow the compiled expression AST.
    pub fn ast(&self) -> &AST {
        &self.ast
    }
    /// Display-friendly file name.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    /// 1-indexed line number.
    pub fn line(&self) -> u32 {
        self.line
    }
    /// The full source text of the rule (verb + expr), for audit-log snapshots.
    pub fn source_text(&self) -> &str {
        &self.source_text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Allow,
    Deny,
    Ask,
}

impl std::fmt::Display for Verb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => f.write_str("allow"),
            Self::Deny => f.write_str("deny"),
            Self::Ask => f.write_str("ask"),
        }
    }
}

/// A parsed, compiled ruleset, grouped by verb for evaluation order.
#[derive(Debug)]
pub struct RuleSet {
    deny_rules: Vec<CompiledRule>,
    ask_rules: Vec<CompiledRule>,
    allow_rules: Vec<CompiledRule>,
}

impl RuleSet {
    /// Parse a ruleset from in-memory source.
    pub fn parse(engine: &Engine, source: &str, file_name: &str) -> Result<Self, ParseError> {
        let mut deny = Vec::new();
        let mut ask = Vec::new();
        let mut allow = Vec::new();

        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = (idx + 1) as u32;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            let rule = parse_rule(engine, line, file_name, line_no)?;
            match rule.verb {
                Verb::Deny => deny.push(rule),
                Verb::Ask => ask.push(rule),
                Verb::Allow => allow.push(rule),
            }
        }

        Ok(Self {
            deny_rules: deny,
            ask_rules: ask,
            allow_rules: allow,
        })
    }

    /// Load and parse a ruleset from disk.
    pub fn load(engine: &Engine, path: &Path) -> Result<Self, ParseError> {
        let source = std::fs::read_to_string(path).map_err(|source| ParseError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self::parse(engine, &source, &name)
    }

    /// Rules with verb `deny`, in source order.
    pub fn deny_rules(&self) -> impl Iterator<Item = &CompiledRule> {
        self.deny_rules.iter()
    }
    /// Rules with verb `ask`, in source order.
    pub fn ask_rules(&self) -> impl Iterator<Item = &CompiledRule> {
        self.ask_rules.iter()
    }
    /// Rules with verb `allow`, in source order.
    pub fn allow_rules(&self) -> impl Iterator<Item = &CompiledRule> {
        self.allow_rules.iter()
    }
}

fn strip_comment(line: &str) -> &str {
    // Strip a trailing `// …` comment, but keep `//` that appears inside a double-quoted
    // string literal intact. Required because users embed `//` inside regex patterns and
    // URLs — e.g. `url.regex("^https?://...")`.
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if escape {
            // Previous byte was a backslash inside a string — consume this byte literally.
            escape = false;
        } else if in_string && c == b'\\' {
            escape = true;
        } else if c == b'"' {
            in_string = !in_string;
        } else if !in_string && c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn parse_rule(
    engine: &Engine,
    line: &str,
    file_name: &str,
    line_no: u32,
) -> Result<CompiledRule, ParseError> {
    // Trim a trailing semicolon if present.
    let trimmed = line.trim_end_matches(';').trim();

    // Split into "<verb> if <expr>".
    let (verb_word, rest) = trimmed
        .split_once(' ')
        .ok_or_else(|| ParseError::Malformed {
            file: file_name.to_owned(),
            line: line_no,
            message: "expected '<verb> if <expression>'".to_owned(),
        })?;

    let verb = match verb_word {
        "allow" => Verb::Allow,
        "deny" => Verb::Deny,
        "ask" => Verb::Ask,
        other => {
            return Err(ParseError::Malformed {
                file: file_name.to_owned(),
                line: line_no,
                message: format!("unknown verb `{other}` (expected allow/deny/ask)"),
            });
        }
    };

    let rest = rest.trim_start();
    let expr = rest
        .strip_prefix("if")
        .ok_or_else(|| ParseError::Malformed {
            file: file_name.to_owned(),
            line: line_no,
            message: "expected `if` after verb".to_owned(),
        })?;
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(ParseError::Malformed {
            file: file_name.to_owned(),
            line: line_no,
            message: "missing expression after `if`".to_owned(),
        });
    }

    // Compile the expression. We compile as a *script* (not just an expression) so users can use
    // top-level boolean logic; the script must return a boolean as its last expression value.
    let ast = engine
        .rhai()
        .compile_expression(expr)
        .map_err(|err| ParseError::BadExpression {
            file: file_name.to_owned(),
            line: line_no,
            message: err.to_string(),
        })?;

    Ok(CompiledRule {
        verb,
        file_name: file_name.to_owned(),
        line: line_no,
        source_text: line.to_owned(),
        ast,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_rule() {
        let eng = Engine::new();
        let rs = RuleSet::parse(&eng, "allow if tool == \"Bash\";", "test").unwrap();
        assert_eq!(rs.deny_rules().count(), 0);
        assert_eq!(rs.ask_rules().count(), 0);
        assert_eq!(rs.allow_rules().count(), 1);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let eng = Engine::new();
        let src = "// header\n\n  // indented\nallow if tool == \"x\";\n";
        let rs = RuleSet::parse(&eng, src, "t").unwrap();
        assert_eq!(rs.allow_rules().count(), 1);
    }

    #[test]
    fn unknown_verb_is_rejected_with_line_number() {
        let eng = Engine::new();
        let src = "allow if tool == \"a\";\nyolo if tool == \"b\";\n";
        let err = RuleSet::parse(&eng, src, "t").unwrap_err();
        match err {
            ParseError::Malformed { line, message, .. } => {
                assert_eq!(line, 2);
                assert!(message.contains("yolo"), "got: {message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn bad_expression_is_rejected_with_line_number() {
        let eng = Engine::new();
        // Syntactically broken Rhai.
        let src = "allow if tool == ;";
        let err = RuleSet::parse(&eng, src, "t").unwrap_err();
        match err {
            ParseError::BadExpression { line, .. } => assert_eq!(line, 1),
            other => panic!("expected BadExpression, got {other:?}"),
        }
    }

    #[test]
    fn comment_stripper_preserves_double_slash_inside_strings() {
        // Regression: default.rhai has `url.regex("^https?://...")` which crashed the
        // parser when strip_comment naively cut at the first `//`.
        let eng = Engine::new();
        let src =
            r#"ask if tool == "WebFetch" && url.regex("^https?://prod\\.");  // trailing comment"#;
        let rs = RuleSet::parse(&eng, src, "t").expect("rule with // inside string must parse");
        assert_eq!(rs.ask_rules().count(), 1);
    }

    #[test]
    fn comment_stripper_handles_escaped_quotes() {
        let eng = Engine::new();
        let src = r#"allow if cmd.contains("she said \"//\" loudly");  // edge case"#;
        let rs = RuleSet::parse(&eng, src, "t").expect("escaped quote inside string");
        assert_eq!(rs.allow_rules().count(), 1);
    }

    #[test]
    fn missing_if_is_rejected() {
        let eng = Engine::new();
        let src = "allow tool == \"x\";";
        let err = RuleSet::parse(&eng, src, "t").unwrap_err();
        match err {
            ParseError::Malformed { message, .. } => {
                assert!(message.contains("`if`"), "got: {message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }
}
