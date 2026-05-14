//! Pattern normalisation for the learning subsystem.
//!
//! The goal: turn `(tool, tool_input, cwd)` into a coarse glob that's likely to match the
//! user's *next* call of the same intent. v0 favours predictability over precision — the
//! patterns we generate are deliberately broad, because surfacing a too-narrow rule fails
//! silently (the next call still asks). A user can always tighten the rule by hand after
//! accepting.
//!
//! Strategy per tool:
//!
//! - **Bash**: keep the first 2 whitespace tokens, suffix with `*`. So `git push origin main`
//!   becomes `git push *`, `npm install foo` becomes `npm install *`.
//! - **Read / Edit / Write**: keep the parent directory of the file, suffix `/*`. So
//!   `/home/rsx/dev/foo.txt` becomes `/home/rsx/dev/*`.
//! - **WebFetch**: extract the URL's host. So `https://example.com/path` becomes
//!   `example.com` (used inside a `url.contains(...)` rule, so substring match is fine).
//! - **Other tools** (including MCP): match by tool name only.

use serde::{Deserialize, Serialize};

/// A normalised pattern ready for hashing + rule generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NormalizedPattern {
    /// Stable hash (xxh3) of `tool|repr_value` for de-dup.
    pub hash: String,
    /// Tool name (unchanged).
    pub tool: String,
    /// The match value: a glob for Bash/Read/Edit/Write, a host substring for WebFetch.
    pub repr_value: String,
    /// Human-readable form for surfacing to the user (e.g. `"Bash: git push *"`).
    pub repr: String,
    /// Working-directory prefix relevant for this observation.
    pub cwd_prefix: String,
}

/// Normalise a tool call into a pattern. See module docs for the strategy per tool.
pub fn normalize_pattern(
    tool: &str,
    tool_input: &serde_json::Value,
    cwd: &str,
) -> NormalizedPattern {
    let repr_value = match tool {
        "Bash" => bash_command_pattern(tool_input),
        "Read" | "Edit" | "Write" => path_pattern(tool_input),
        "WebFetch" => url_host(tool_input),
        _ => String::new(),
    };
    let repr = if repr_value.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool}: {repr_value}")
    };
    let hash_input = format!("{tool}|{repr_value}");
    let hash = format!("{:x}", xxhash_rust::xxh3::xxh3_64(hash_input.as_bytes()));
    NormalizedPattern {
        hash,
        tool: tool.to_owned(),
        repr_value,
        repr,
        cwd_prefix: cwd_prefix_for(cwd),
    }
}

fn bash_command_pattern(input: &serde_json::Value) -> String {
    let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut parts = trimmed.split_whitespace();
    let first = match parts.next() {
        Some(s) => s,
        None => return String::new(),
    };
    let second = parts.next();
    match second {
        Some(s) if !s.starts_with('-') => format!("{first} {s} *"),
        Some(_) | None => format!("{first} *"),
    }
}

fn path_pattern(input: &serde_json::Value) -> String {
    let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if path.is_empty() {
        return String::new();
    }
    // Find the last `/`. If none, just glob.
    match path.rfind('/') {
        Some(idx) => format!("{}/*", &path[..idx]),
        None => "*".to_owned(),
    }
}

fn url_host(input: &serde_json::Value) -> String {
    let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if url.is_empty() {
        return String::new();
    }
    // Cheap-and-cheerful host extraction: between "//" and the next "/" or "?" or end.
    let after_scheme = url.split_once("//").map(|(_, b)| b).unwrap_or(url);
    let host: String = after_scheme
        .chars()
        .take_while(|c| !matches!(c, '/' | '?' | '#'))
        .collect();
    host
}

fn cwd_prefix_for(cwd: &str) -> String {
    // For now: keep the cwd verbatim. We could collapse to the project root later, but
    // since we don't filter by cwd at the rule level, the prefix is informational.
    cwd.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(cmd: &str) -> serde_json::Value {
        json!({"command": cmd})
    }

    #[test]
    fn bash_pattern_keeps_first_two_tokens() {
        let p = normalize_pattern("Bash", &input("npm install foo"), "/x");
        assert_eq!(p.repr_value, "npm install *");
        let p = normalize_pattern("Bash", &input("git push origin main"), "/x");
        assert_eq!(p.repr_value, "git push *");
        let p = normalize_pattern("Bash", &input("ls -la /tmp"), "/x");
        assert_eq!(
            p.repr_value, "ls *",
            "flag in second pos collapses to one-token"
        );
    }

    #[test]
    fn bash_pattern_handles_single_token() {
        let p = normalize_pattern("Bash", &input("ls"), "/x");
        assert_eq!(p.repr_value, "ls *");
    }

    #[test]
    fn bash_pattern_empty_command_yields_empty() {
        let p = normalize_pattern("Bash", &input(""), "/x");
        assert_eq!(p.repr_value, "");
        assert_eq!(p.repr, "Bash");
    }

    #[test]
    fn path_pattern_globs_parent_directory() {
        let p = normalize_pattern(
            "Read",
            &json!({"path": "/home/rsx/dev/foo.txt"}),
            "/home/rsx/dev",
        );
        assert_eq!(p.repr_value, "/home/rsx/dev/*");
    }

    #[test]
    fn url_pattern_extracts_host() {
        let p = normalize_pattern(
            "WebFetch",
            &json!({"url": "https://example.com/some/path?q=1"}),
            "/x",
        );
        assert_eq!(p.repr_value, "example.com");
    }

    #[test]
    fn url_pattern_handles_no_scheme() {
        let p = normalize_pattern("WebFetch", &json!({"url": "example.org"}), "/x");
        assert_eq!(p.repr_value, "example.org");
    }

    #[test]
    fn unknown_tool_yields_tool_name_repr() {
        let p = normalize_pattern("WeirdTool", &json!({"x": "y"}), "/x");
        assert_eq!(p.repr_value, "");
        assert_eq!(p.repr, "WeirdTool");
    }

    #[test]
    fn same_input_yields_same_hash() {
        let a = normalize_pattern("Bash", &input("npm install foo"), "/x");
        let b = normalize_pattern("Bash", &input("npm install bar"), "/y");
        // Both normalise to `npm install *`, so hashes should match.
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn different_tools_have_different_hashes() {
        let a = normalize_pattern("Bash", &input("npm install foo"), "/x");
        let b = normalize_pattern("Read", &json!({"path": "/x/npm-install-foo"}), "/x");
        assert_ne!(a.hash, b.hash);
    }
}
