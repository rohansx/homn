//! Install + uninstall the homn hook entries in `~/.claude/settings.json`.
//!
//! Implementing T080. Two modes:
//!
//! - **Print** (`homn install`): write the recommended JSON snippet to stdout. Idempotent — the
//!   snippet is the same every call. The user pastes it into their settings.json.
//! - **Apply** (`homn install --apply`): read settings.json, merge in the homn hook entries
//!   without disturbing any other hooks, back up the original, write atomically.
//!
//! The merge is idempotent: re-running `--apply` is a no-op if our entry is already present.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Stable identifier we mark our own hook entries with so we can recognise them on re-install.
const HOMN_MARKER: &str = "/* installed by homn */";

/// The Claude Code hook entry we install for `PermissionRequest`. Matches
/// [`specs/001-policy-engine/contracts/hook-protocol.md`].
pub fn permission_request_entry() -> Value {
    json!({
        "matcher": "*",
        "_homn": HOMN_MARKER,
        "hooks": [{
            "type": "command",
            "command": "homn hook permission-request",
            "timeout": 30000
        }]
    })
}

/// The full snippet to merge into `~/.claude/settings.json`. Only `PermissionRequest` is wired
/// in v0; SessionStart/UserPromptSubmit/Stop/Notification land alongside Phase 3.
pub fn install_snippet() -> Value {
    json!({
        "hooks": {
            "PermissionRequest": [permission_request_entry()]
        }
    })
}

/// What the install operation did. Returned so the caller can render a friendly message.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallReport {
    /// Snippet printed to caller; nothing on disk changed.
    Printed,
    /// Apply mode: created a new settings.json file.
    CreatedNew {
        /// The settings.json path that was written.
        path: PathBuf,
    },
    /// Apply mode: merged into an existing file; backup written.
    MergedExisting {
        /// The settings.json path that was updated.
        path: PathBuf,
        /// Where the original was backed up before the merge.
        backup: PathBuf,
    },
    /// Apply mode: nothing to do — entry was already present.
    AlreadyPresent {
        /// The settings.json path that was inspected.
        path: PathBuf,
    },
}

/// Find `~/.claude/settings.json` by `$HOME`.
pub fn default_settings_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude").join("settings.json")
}

/// Print-or-apply install. See module docs.
pub fn run_install(
    settings_path: &Path,
    apply: bool,
    output: &mut dyn std::io::Write,
) -> anyhow::Result<InstallReport> {
    if !apply {
        let pretty = serde_json::to_string_pretty(&install_snippet())?;
        writeln!(
            output,
            "# Paste this snippet into ~/.claude/settings.json (merge with any existing hooks):"
        )?;
        writeln!(output, "{pretty}")?;
        writeln!(
            output,
            "\n# Or run `homn install --apply` to do the merge automatically (a backup is written)."
        )?;
        return Ok(InstallReport::Printed);
    }

    // --apply path
    if !settings_path.exists() {
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            settings_path,
            serde_json::to_string_pretty(&install_snippet())?,
        )?;
        return Ok(InstallReport::CreatedNew {
            path: settings_path.to_path_buf(),
        });
    }

    let original_text = std::fs::read_to_string(settings_path)?;
    let mut value: Value = if original_text.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(&original_text)?
    };

    let already = is_homn_entry_present(&value);
    if already {
        return Ok(InstallReport::AlreadyPresent {
            path: settings_path.to_path_buf(),
        });
    }

    merge_install_snippet(&mut value);

    // Atomic write: write to temp then rename. Back up the original first.
    let backup_path = backup_path_for(settings_path);
    std::fs::copy(settings_path, &backup_path)?;
    let tmp_path = settings_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&value)?)?;
    std::fs::rename(&tmp_path, settings_path)?;
    Ok(InstallReport::MergedExisting {
        path: settings_path.to_path_buf(),
        backup: backup_path,
    })
}

/// Mutates `settings` to include the homn `PermissionRequest` hook entry, preserving any
/// existing hooks at that key. Safe to call on a value that already contains our entry — in
/// that case nothing changes.
pub fn merge_install_snippet(settings: &mut Value) {
    let obj = match settings {
        Value::Object(m) => m,
        _ => {
            // Replace whatever was there with an empty object first.
            *settings = Value::Object(serde_json::Map::new());
            if let Value::Object(m) = settings {
                m
            } else {
                unreachable!()
            }
        }
    };

    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !hooks.is_object() {
        *hooks = Value::Object(serde_json::Map::new());
    }
    let hooks_obj = hooks.as_object_mut().expect("hooks is object");
    let pr = hooks_obj
        .entry("PermissionRequest")
        .or_insert_with(|| Value::Array(vec![]));
    if !pr.is_array() {
        *pr = Value::Array(vec![]);
    }
    let arr = pr.as_array_mut().expect("PermissionRequest is array");
    if !arr.iter().any(is_homn_entry) {
        arr.push(permission_request_entry());
    }
}

/// True if any installed PermissionRequest hook is ours.
pub fn is_homn_entry_present(settings: &Value) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get("PermissionRequest"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(is_homn_entry))
        .unwrap_or(false)
}

fn is_homn_entry(v: &Value) -> bool {
    v.get("_homn").and_then(|m| m.as_str()) == Some(HOMN_MARKER)
}

/// True if a `PermissionRequest` entry is one homn installed.
/// Matches by the `_homn` marker field (set by `--apply`) or by a command
/// containing `"homn hook"` (set by hand-added entries).
fn entry_is_homn(entry: &Value) -> bool {
    // Check the `_homn` marker field first (install path stamps this).
    if entry.get("_homn").and_then(|m| m.as_str()) == Some(HOMN_MARKER) {
        return true;
    }
    // Fall back to command-string match for hand-added entries.
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains("homn hook"))
            })
        })
        .unwrap_or(false)
}

/// Remove homn's `PermissionRequest` hook entry from a parsed `settings.json`.
/// Returns `true` if anything was removed; other hooks are left untouched.
pub fn remove_homn_entry(settings: &mut Value) -> bool {
    let Some(arr) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PermissionRequest"))
        .and_then(|pr| pr.as_array_mut())
    else {
        return false;
    };
    let before = arr.len();
    arr.retain(|entry| !entry_is_homn(entry));
    before != arr.len()
}

fn backup_path_for(settings_path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = settings_path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(format!(".bak.{ts}"));
    settings_path
        .parent()
        .map(|p| p.join(name.clone()))
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn snippet_contains_permission_request_command() {
        let s = install_snippet();
        assert_eq!(
            s["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "homn hook permission-request"
        );
        assert_eq!(s["hooks"]["PermissionRequest"][0]["matcher"], "*");
    }

    #[test]
    fn merge_into_empty_object_creates_full_structure() {
        let mut v = json!({});
        merge_install_snippet(&mut v);
        assert!(is_homn_entry_present(&v));
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "homn hook permission-request"
        );
    }

    #[test]
    fn merge_preserves_unrelated_keys() {
        let mut v = json!({"otherKey": "untouched"});
        merge_install_snippet(&mut v);
        assert_eq!(v["otherKey"], "untouched");
        assert!(is_homn_entry_present(&v));
    }

    #[test]
    fn merge_preserves_existing_permission_request_entries() {
        let mut v = json!({
            "hooks": {
                "PermissionRequest": [
                    { "matcher": "Bash", "hooks": [{"type": "command", "command": "someone-elses-hook"}] }
                ]
            }
        });
        merge_install_snippet(&mut v);
        let arr = v["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "should append, not replace");
        assert_eq!(arr[0]["hooks"][0]["command"], "someone-elses-hook");
        assert_eq!(
            arr[1]["hooks"][0]["command"],
            "homn hook permission-request"
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let mut v = json!({});
        merge_install_snippet(&mut v);
        merge_install_snippet(&mut v);
        merge_install_snippet(&mut v);
        let arr = v["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "expected exactly one homn entry after 3 merges"
        );
    }

    #[test]
    fn print_mode_emits_snippet_to_writer() {
        let mut out = Cursor::new(Vec::<u8>::new());
        let report = run_install(Path::new("/nonexistent"), false, &mut out).unwrap();
        assert_eq!(report, InstallReport::Printed);
        let s = String::from_utf8(out.into_inner()).unwrap();
        assert!(s.contains("homn hook permission-request"));
        assert!(s.contains("PermissionRequest"));
    }

    #[test]
    fn apply_mode_creates_new_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let mut out = Cursor::new(Vec::<u8>::new());
        let report = run_install(&path, true, &mut out).unwrap();
        assert_eq!(report, InstallReport::CreatedNew { path: path.clone() });
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(is_homn_entry_present(&written));
    }

    #[test]
    fn apply_mode_merges_existing_file_and_writes_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({"otherKey": "untouched"})).unwrap(),
        )
        .unwrap();

        let mut out = Cursor::new(Vec::<u8>::new());
        let report = run_install(&path, true, &mut out).unwrap();
        match report {
            InstallReport::MergedExisting { backup, .. } => {
                assert!(backup.exists(), "backup file should exist");
                let backup_value: Value =
                    serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
                assert_eq!(backup_value["otherKey"], "untouched");
            }
            other => panic!("expected MergedExisting, got {other:?}"),
        }
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(is_homn_entry_present(&merged));
        assert_eq!(merged["otherKey"], "untouched");
    }

    #[test]
    fn remove_homn_entry_drops_only_homn_and_keeps_others() {
        let mut settings = serde_json::json!({
            "hooks": {
                "PermissionRequest": [
                    { "matcher": "Bash", "hooks": [
                        { "type": "command", "command": "someone-elses-hook" } ] },
                    { "matcher": "*", "hooks": [
                        { "type": "command", "command": "homn hook permission-request" } ] }
                ]
            }
        });
        assert!(remove_homn_entry(&mut settings), "removes the homn entry");
        let arr = settings["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "the non-homn hook survives");
        assert_eq!(arr[0]["hooks"][0]["command"], "someone-elses-hook");
        assert!(
            !remove_homn_entry(&mut settings),
            "second removal is a no-op (idempotent)"
        );
    }

    #[test]
    fn remove_homn_entry_drops_marker_stamped_entry() {
        // Entries installed by `homn install --apply` carry the `_homn` marker field.
        // entry_is_homn must match by marker alone (without relying on the command string).
        let mut settings = serde_json::json!({
            "hooks": {
                "PermissionRequest": [
                    { "matcher": "other", "hooks": [
                        { "type": "command", "command": "other-hook" } ] },
                    {
                        "matcher": "*",
                        "_homn": "/* installed by homn */",
                        "hooks": [
                            { "type": "command", "command": "/custom/path/to/homn-wrapper", "timeout": 30000 }
                        ]
                    }
                ]
            }
        });
        assert!(remove_homn_entry(&mut settings), "marker-stamped entry is removed even when command doesn't contain 'homn hook'");
        let arr = settings["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "unrelated hook survives");
        assert_eq!(arr[0]["hooks"][0]["command"], "other-hook");
    }

    #[test]
    fn apply_mode_idempotent_on_second_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut out = Cursor::new(Vec::<u8>::new());

        let first = run_install(&path, true, &mut out).unwrap();
        assert!(matches!(first, InstallReport::CreatedNew { .. }));

        let second = run_install(&path, true, &mut out).unwrap();
        assert!(matches!(second, InstallReport::AlreadyPresent { .. }));
    }
}
