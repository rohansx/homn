# Easy Install & Setup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a new user install homn and get it guarding Claude Code in two commands — `curl … | sh` then `homn setup`.

**Architecture:** Three independent pieces. (A) A `homn-hook/src/setup.rs` module with pure, unit-tested functions (policy seeding, service-file generation, hook removal) plus thin `systemctl`/`launchctl` wrappers, driven by new `homn setup` / `homn uninstall` CLI subcommands. (B) A `curl | sh` `install.sh` that downloads a verified prebuilt binary. (C) A GitHub Actions release workflow that cross-compiles and publishes those binaries.

**Tech Stack:** Rust (stable), `serde_json`, `tempfile` (dev), Bash, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-05-18-easy-install-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/homn-hook/Cargo.toml` | Modify | add `homn-policy` dep (parse-check), ensure `tempfile` dev-dep |
| `crates/homn-hook/src/lib.rs` | Modify | `pub mod setup;` |
| `crates/homn-hook/src/setup.rs` | Create | policy seeding, service-file generation, init detection, `run_setup`/`run_uninstall` |
| `crates/homn-hook/src/install.rs` | Modify | add `remove_homn_entry` (uninstall counterpart of the merge) |
| `crates/homn-bin/src/main.rs` | Modify | `Setup` + `Uninstall` subcommands + handlers |
| `install.sh` | Create | `curl \| sh` installer |
| `.github/workflows/release.yml` | Create | cross-compile + publish binaries on tag |
| `.github/workflows/ci.yml` | Modify | add a `shellcheck` job for `install.sh` |
| `README.md` | Modify | two-line quick-start |
| `docs/getting-started.md` | Modify | two-line quick-start |

---

## Task 1: Policy seeding in a new `setup` module

**Files:**
- Modify: `crates/homn-hook/Cargo.toml`
- Modify: `crates/homn-hook/src/lib.rs:20-21`
- Create: `crates/homn-hook/src/setup.rs`

- [ ] **Step 1: Add the `homn-policy` dependency**

In `crates/homn-hook/Cargo.toml`, under `[dependencies]`, add after the `homn-audit` line:

```toml
homn-policy        = { workspace = true }
```

Under `[dev-dependencies]`, ensure this line exists (add it if absent):

```toml
tempfile           = { workspace = true }
```

- [ ] **Step 2: Declare the module**

In `crates/homn-hook/src/lib.rs`, after the line `pub mod pty;` (line 21), add:

```rust
pub mod setup;
```

- [ ] **Step 3: Write the failing test**

Create `crates/homn-hook/src/setup.rs` with this content:

```rust
//! `homn setup` / `homn uninstall` orchestration.
//!
//! `setup` takes a fresh install from "binary on disk" to "daemon running, hook wired,
//! policy seeded" in one idempotent command; `uninstall` reverses it. The pure,
//! side-effect-free pieces (policy seeding, service-file generation, hook removal) live
//! here and are unit-tested; the thin `systemctl` / `launchctl` calls are isolated.

use std::path::{Path, PathBuf};

/// Which bundled policy profile `homn setup` seeds when no policy exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyProfile {
    /// Balanced — denies the destructive, asks the high-stakes, allows the dev loop.
    Default,
    /// Locked down — read-only allows, everything else asked.
    Strict,
    /// Trusted — full dev loop, only the irreversible denied.
    Relaxed,
}

impl PolicyProfile {
    /// The bundled `.rhai` text for this profile, baked in at compile time.
    pub fn text(self) -> &'static str {
        match self {
            PolicyProfile::Default => include_str!("../../../policies/default.rhai"),
            PolicyProfile::Strict => include_str!("../../../policies/strict.rhai"),
            PolicyProfile::Relaxed => include_str!("../../../policies/relaxed.rhai"),
        }
    }
}

/// Result of [`seed_policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySeedOutcome {
    /// No policy existed; the bundled profile was written to this path.
    Written(PathBuf),
    /// A policy already existed and parses cleanly; left untouched.
    KeptExisting(PathBuf),
    /// A policy already existed but does NOT parse; left untouched — caller should warn.
    KeptUnparseable(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_policy_writes_the_profile_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let policies = dir.path().join("policies");
        let outcome = seed_policy(&policies, PolicyProfile::Default).unwrap();
        let target = policies.join("default.rhai");
        assert_eq!(outcome, PolicySeedOutcome::Written(target.clone()));
        assert!(target.exists());
        let engine = homn_policy::Engine::new();
        homn_policy::RuleSet::parse(
            &engine,
            &std::fs::read_to_string(&target).unwrap(),
            "default.rhai",
        )
        .expect("the seeded policy must parse");
    }

    #[test]
    fn seed_policy_keeps_an_existing_valid_policy() {
        let dir = tempfile::tempdir().unwrap();
        let policies = dir.path().join("policies");
        std::fs::create_dir_all(&policies).unwrap();
        let target = policies.join("default.rhai");
        std::fs::write(&target, "allow if tool == \"Read\";\n").unwrap();
        let outcome = seed_policy(&policies, PolicyProfile::Default).unwrap();
        assert_eq!(outcome, PolicySeedOutcome::KeptExisting(target.clone()));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "allow if tool == \"Read\";\n",
            "an existing valid policy is left byte-for-byte unchanged"
        );
    }

    #[test]
    fn seed_policy_flags_but_keeps_a_broken_existing_policy() {
        let dir = tempfile::tempdir().unwrap();
        let policies = dir.path().join("policies");
        std::fs::create_dir_all(&policies).unwrap();
        let target = policies.join("default.rhai");
        std::fs::write(&target, "broken = = =\n").unwrap();
        let outcome = seed_policy(&policies, PolicyProfile::Default).unwrap();
        assert_eq!(outcome, PolicySeedOutcome::KeptUnparseable(target.clone()));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "broken = = =\n",
            "a broken policy is flagged but never clobbered"
        );
    }
}
```

- [ ] **Step 4: Run the test, verify it fails**

Run: `cargo test -p homn-hook seed_policy`
Expected: FAIL — compile error, `cannot find function seed_policy in this scope`.

- [ ] **Step 5: Implement `seed_policy`**

In `crates/homn-hook/src/setup.rs`, insert before the `#[cfg(test)]` line:

```rust
/// Ensure `<policies_dir>/default.rhai` exists. Idempotent and non-destructive: an
/// existing policy is never overwritten, even if it fails to parse.
pub fn seed_policy(
    policies_dir: &Path,
    profile: PolicyProfile,
) -> std::io::Result<PolicySeedOutcome> {
    let target = policies_dir.join("default.rhai");
    if target.exists() {
        let text = std::fs::read_to_string(&target)?;
        let engine = homn_policy::Engine::new();
        return Ok(
            match homn_policy::RuleSet::parse(&engine, &text, "default.rhai") {
                Ok(_) => PolicySeedOutcome::KeptExisting(target),
                Err(_) => PolicySeedOutcome::KeptUnparseable(target),
            },
        );
    }
    std::fs::create_dir_all(policies_dir)?;
    std::fs::write(&target, profile.text())?;
    Ok(PolicySeedOutcome::Written(target))
}
```

- [ ] **Step 6: Run the test, verify it passes**

Run: `cargo test -p homn-hook seed_policy`
Expected: PASS — 3 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/homn-hook/Cargo.toml crates/homn-hook/src/lib.rs crates/homn-hook/src/setup.rs
git commit -m "feat(setup): policy seeding for homn setup (idempotent, non-destructive)"
```

---

## Task 2: Service-file generators

**Files:**
- Modify: `crates/homn-hook/src/setup.rs`

- [ ] **Step 1: Write the failing tests**

In `crates/homn-hook/src/setup.rs`, add inside `mod tests` (before the closing `}`):

```rust
    #[test]
    fn systemd_unit_embeds_the_resolved_binary_path() {
        let unit = systemd_unit(Path::new("/home/u/.local/bin/homn"));
        assert!(
            unit.contains("ExecStart=/home/u/.local/bin/homn daemon --foreground"),
            "ExecStart must use the resolved absolute path:\n{unit}"
        );
        assert!(
            !unit.contains("%h/.cargo/bin/homn"),
            "the template placeholder must be gone"
        );
        assert!(unit.contains("[Install]"), "still a complete unit file");
    }

    #[test]
    fn launchd_plist_embeds_the_resolved_binary_path() {
        let plist = launchd_plist(Path::new("/Users/u/.local/bin/homn"));
        assert!(plist.contains("<string>/Users/u/.local/bin/homn</string>"));
        assert!(plist.contains("sh.homn.daemon"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test -p homn-hook -- systemd_unit launchd_plist`
Expected: FAIL — `cannot find function systemd_unit` / `launchd_plist`.

- [ ] **Step 3: Implement the generators**

In `crates/homn-hook/src/setup.rs`, add after `seed_policy`:

```rust
/// Generate the `systemd --user` unit text for the daemon at `exec_path`.
///
/// Sourced from the committed `dist/homn.service` template (the single source of truth
/// for the hardening directives) with the `ExecStart` path rewritten to `exec_path`.
pub fn systemd_unit(exec_path: &Path) -> String {
    const TEMPLATE: &str = include_str!("../../../dist/homn.service");
    TEMPLATE.replace("%h/.cargo/bin/homn", &exec_path.display().to_string())
}

/// Generate a launchd LaunchAgent plist for the daemon at `exec_path` (macOS).
pub fn launchd_plist(exec_path: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>sh.homn.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
        <string>daemon</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        exec = exec_path.display(),
    )
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test -p homn-hook -- systemd_unit launchd_plist`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/homn-hook/src/setup.rs
git commit -m "feat(setup): systemd unit + launchd plist generators"
```

---

## Task 3: Hook removal for uninstall

**Files:**
- Modify: `crates/homn-hook/src/install.rs`

- [ ] **Step 1: Write the failing test**

In `crates/homn-hook/src/install.rs`, inside the `#[cfg(test)] mod tests` block, add:

```rust
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
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p homn-hook remove_homn_entry`
Expected: FAIL — `cannot find function remove_homn_entry`.

- [ ] **Step 3: Implement `remove_homn_entry`**

In `crates/homn-hook/src/install.rs`, add after `is_homn_entry_present` (around line 174):

```rust
/// True if a `PermissionRequest` entry is one homn installed.
fn entry_is_homn(entry: &Value) -> bool {
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
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo test -p homn-hook remove_homn_entry`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/homn-hook/src/install.rs
git commit -m "feat(setup): remove_homn_entry — uninstall counterpart of the hook merge"
```

---

## Task 4: Init-system detection + service wrappers

**Files:**
- Modify: `crates/homn-hook/src/setup.rs`

- [ ] **Step 1: Write the failing test**

In `crates/homn-hook/src/setup.rs`, add inside `mod tests`:

```rust
    #[test]
    #[cfg(target_os = "linux")]
    fn detect_init_system_is_systemd_on_linux() {
        assert_eq!(detect_init_system(), InitSystem::Systemd);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn detect_init_system_is_launchd_on_macos() {
        assert_eq!(detect_init_system(), InitSystem::Launchd);
    }
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p homn-hook detect_init_system`
Expected: FAIL — `cannot find type InitSystem` / `function detect_init_system`.

- [ ] **Step 3: Implement detection + the service wrappers**

In `crates/homn-hook/src/setup.rs`, add after the generators:

```rust
/// Which service manager `homn setup` registers the daemon with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSystem {
    /// Linux with systemd — a `--user` unit.
    Systemd,
    /// macOS — a launchd LaunchAgent.
    Launchd,
    /// Neither — `homn setup` prints manual instructions.
    Unsupported,
}

/// Detect the host's service manager.
pub fn detect_init_system() -> InitSystem {
    if cfg!(target_os = "linux") {
        InitSystem::Systemd
    } else if cfg!(target_os = "macos") {
        InitSystem::Launchd
    } else {
        InitSystem::Unsupported
    }
}

/// Run a command, returning an error on non-zero exit.
fn run(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new(cmd).args(args).status()?;
    if !status.success() {
        anyhow::bail!("`{cmd} {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME/systemd/user`, or `~/.config/systemd/user`.
fn systemd_user_dir() -> anyhow::Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("systemd").join("user"));
        }
    }
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

/// `~/Library/LaunchAgents/sh.homn.daemon.plist`.
fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents/sh.homn.daemon.plist"))
}

/// Write + enable + start the systemd `--user` service. Returns the unit path.
pub fn install_systemd_service(exec_path: &Path) -> anyhow::Result<PathBuf> {
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir)?;
    let unit_path = dir.join("homn.service");
    std::fs::write(&unit_path, systemd_unit(exec_path))?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", "homn.service"])?;
    Ok(unit_path)
}

/// Stop + disable + remove the systemd `--user` service. Safe if not installed.
pub fn remove_systemd_service() -> anyhow::Result<()> {
    let _ = run("systemctl", &["--user", "disable", "--now", "homn.service"]);
    if let Ok(unit) = systemd_user_dir().map(|d| d.join("homn.service")) {
        if unit.exists() {
            std::fs::remove_file(unit)?;
        }
    }
    let _ = run("systemctl", &["--user", "daemon-reload"]);
    Ok(())
}

/// Write + load the launchd LaunchAgent. Returns the plist path.
pub fn install_launchd_service(exec_path: &Path) -> anyhow::Result<PathBuf> {
    let plist_path = launchd_plist_path()?;
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, launchd_plist(exec_path))?;
    let _ = run("launchctl", &["unload", &plist_path.display().to_string()]);
    run("launchctl", &["load", &plist_path.display().to_string()])?;
    Ok(plist_path)
}

/// Unload + remove the launchd LaunchAgent. Safe if not installed.
pub fn remove_launchd_service() -> anyhow::Result<()> {
    if let Ok(plist_path) = launchd_plist_path() {
        let _ = run("launchctl", &["unload", &plist_path.display().to_string()]);
        if plist_path.exists() {
            std::fs::remove_file(plist_path)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo test -p homn-hook detect_init_system`
Expected: PASS — 1 test (the one matching this OS).

- [ ] **Step 5: Commit**

```bash
git add crates/homn-hook/src/setup.rs
git commit -m "feat(setup): init-system detection + systemd/launchd service wrappers"
```

---

## Task 5: `run_setup` orchestration

**Files:**
- Modify: `crates/homn-hook/src/setup.rs`

- [ ] **Step 1: Write the failing test**

In `crates/homn-hook/src/setup.rs`, add inside `mod tests`:

```rust
    #[test]
    fn run_setup_without_service_seeds_policy_and_installs_hook() {
        let dir = tempfile::tempdir().unwrap();
        let policies = dir.path().join("config/homn/policies");
        let settings = dir.path().join("claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

        let report = run_setup(SetupOptions {
            policies_dir: policies.clone(),
            settings_path: settings.clone(),
            profile: PolicyProfile::Default,
            install_service: false,
        })
        .unwrap();

        assert!(matches!(report.policy, PolicySeedOutcome::Written(_)));
        assert!(matches!(report.service, ServiceOutcome::SkippedByFlag));
        assert!(policies.join("default.rhai").exists(), "policy seeded");

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(
            crate::install::is_homn_entry_present(&written),
            "the homn hook is now in settings.json"
        );
    }
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p homn-hook run_setup`
Expected: FAIL — `cannot find type SetupOptions` / `function run_setup`.

- [ ] **Step 3: Implement `run_setup` + its types**

In `crates/homn-hook/src/setup.rs`, add these `use` lines near the top of the file with the existing `use std::path::{Path, PathBuf};` import:

```rust
use crate::install::run_install;
use crate::InstallReport;
```

Then add after the service wrappers:

```rust
/// Inputs for [`run_setup`]. Paths are passed in (resolved by the CLI from daemon config)
/// so this function is fully testable against temp directories.
pub struct SetupOptions {
    /// Directory that holds `default.rhai` (the daemon's configured policies dir).
    pub policies_dir: PathBuf,
    /// Path to Claude Code's `settings.json`.
    pub settings_path: PathBuf,
    /// Profile to seed when no policy exists yet.
    pub profile: PolicyProfile,
    /// Whether to install + start the background service.
    pub install_service: bool,
}

/// What happened to the background service during setup.
#[derive(Debug)]
pub enum ServiceOutcome {
    /// Installed + started; here is the unit/plist path.
    Installed(PathBuf),
    /// `--no-service` was passed.
    SkippedByFlag,
    /// Host has no supported service manager — manual steps needed.
    UnsupportedPlatform,
}

/// What [`run_setup`] did, for the CLI to report.
pub struct SetupReport {
    /// Outcome of policy seeding.
    pub policy: PolicySeedOutcome,
    /// Outcome of the Claude Code hook install.
    pub hook: InstallReport,
    /// Outcome of service installation.
    pub service: ServiceOutcome,
}

/// Run `homn setup`: seed the policy, install the hook, install the service. Idempotent.
pub fn run_setup(opts: SetupOptions) -> anyhow::Result<SetupReport> {
    let policy = seed_policy(&opts.policies_dir, opts.profile)?;

    let mut sink = std::io::sink();
    let hook = run_install(&opts.settings_path, true, &mut sink)?;

    let service = if !opts.install_service {
        ServiceOutcome::SkippedByFlag
    } else {
        let exec = std::env::current_exe()?;
        match detect_init_system() {
            InitSystem::Systemd => ServiceOutcome::Installed(install_systemd_service(&exec)?),
            InitSystem::Launchd => ServiceOutcome::Installed(install_launchd_service(&exec)?),
            InitSystem::Unsupported => ServiceOutcome::UnsupportedPlatform,
        }
    };

    Ok(SetupReport {
        policy,
        hook,
        service,
    })
}
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo test -p homn-hook run_setup`
Expected: PASS.

If `is_homn_entry_present` is not `pub`, make it `pub` in `crates/homn-hook/src/install.rs` (change `fn is_homn_entry_present` to `pub fn is_homn_entry_present`).

- [ ] **Step 5: Commit**

```bash
git add crates/homn-hook/src/setup.rs crates/homn-hook/src/install.rs
git commit -m "feat(setup): run_setup — seed policy, install hook, install service"
```

---

## Task 6: `run_uninstall` orchestration

**Files:**
- Modify: `crates/homn-hook/src/setup.rs`

- [ ] **Step 1: Write the failing test**

In `crates/homn-hook/src/setup.rs`, add inside `mod tests`:

```rust
    #[test]
    fn run_uninstall_removes_the_hook_without_touching_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{"hooks":{"PermissionRequest":[
                {"matcher":"*","hooks":[
                    {"type":"command","command":"homn hook permission-request"}]}]}}"#,
        )
        .unwrap();

        let report = run_uninstall(&settings, false).unwrap();
        assert!(report.hook_removed);
        assert!(!report.service_removed, "remove_service=false skips the service");

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(!crate::install::is_homn_entry_present(&after));
    }
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p homn-hook run_uninstall`
Expected: FAIL — `cannot find function run_uninstall`.

- [ ] **Step 3: Implement `run_uninstall`**

In `crates/homn-hook/src/setup.rs`, add this `use` line near the top with the other imports:

```rust
use crate::install::remove_homn_entry;
```

Then add after `run_setup`:

```rust
/// What [`run_uninstall`] did, for the CLI to report.
pub struct UninstallReport {
    /// Whether homn's hook entry was removed from `settings.json`.
    pub hook_removed: bool,
    /// Whether the background service was stopped + removed.
    pub service_removed: bool,
}

/// Run `homn uninstall`: remove the hook, and (if `remove_service`) the service.
/// Policy files and the audit DB are left in place — that is the user's data.
pub fn run_uninstall(settings_path: &Path, remove_service: bool) -> anyhow::Result<UninstallReport> {
    let service_removed = if remove_service {
        match detect_init_system() {
            InitSystem::Systemd => {
                remove_systemd_service()?;
                true
            }
            InitSystem::Launchd => {
                remove_launchd_service()?;
                true
            }
            InitSystem::Unsupported => false,
        }
    } else {
        false
    };

    let hook_removed = if settings_path.exists() {
        let mut settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path)?)?;
        let removed = remove_homn_entry(&mut settings);
        if removed {
            std::fs::write(settings_path, serde_json::to_string_pretty(&settings)?)?;
        }
        removed
    } else {
        false
    };

    Ok(UninstallReport {
        hook_removed,
        service_removed,
    })
}
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo test -p homn-hook run_uninstall`
Expected: PASS.

- [ ] **Step 5: Run the full crate test suite + clippy**

Run: `cargo test -p homn-hook && cargo clippy -p homn-hook --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/homn-hook/src/setup.rs
git commit -m "feat(setup): run_uninstall — reverse setup, keep user data"
```

---

## Task 7: `homn setup` / `homn uninstall` CLI subcommands

**Files:**
- Modify: `crates/homn-bin/src/main.rs`

- [ ] **Step 1: Add the subcommand variants**

In `crates/homn-bin/src/main.rs`, in the `enum Command` block, add after the `Hook { … }` variant (before the closing `}` of the enum):

```rust
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
```

- [ ] **Step 2: Add the handlers**

In `crates/homn-bin/src/main.rs`, in the `match cli.command` block, add before `None => {`:

```rust
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
```

- [ ] **Step 3: Add the report printer**

In `crates/homn-bin/src/main.rs`, at the end of the file, add:

```rust
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
```

- [ ] **Step 4: Build + verify the CLI parses**

Run: `cargo build -p homn-bin && ./target/debug/homn setup --help && ./target/debug/homn uninstall --help`
Expected: builds clean; both help texts print with the documented flags.

- [ ] **Step 5: Commit**

```bash
git add crates/homn-bin/src/main.rs
git commit -m "feat(cli): homn setup + homn uninstall subcommands"
```

---

## Task 8: `install.sh`

**Files:**
- Create: `install.sh`

- [ ] **Step 1: Write the installer**

Create `install.sh` at the repo root with this content:

```bash
#!/usr/bin/env sh
# homn installer — downloads a verified prebuilt binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
#
# Flags (pass after `-s --` when piping, e.g. `... | sh -s -- --version v0.1.0`):
#   --version vX.Y.Z   install a specific release (default: latest)
#   --bin-dir DIR      install location (default: ~/.local/bin)
set -eu

REPO="rohansx/homn"
VERSION=""
BIN_DIR="${HOMN_BIN_DIR:-$HOME/.local/bin}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --bin-dir) BIN_DIR="$2"; shift 2 ;;
        *) echo "homn install: unknown flag '$1'" >&2; exit 1 ;;
    esac
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "homn install: missing '$1'" >&2; exit 1; }; }
need curl
need tar

# --- detect platform -> Rust target triple ---------------------------------
detect_triple() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)  os_part="unknown-linux-gnu" ;;
        Darwin) os_part="apple-darwin" ;;
        *) echo "homn install: unsupported OS '$os' — build from source: cargo install --git https://github.com/$REPO homn-bin" >&2; exit 1 ;;
    esac
    case "$arch" in
        x86_64|amd64)  arch_part="x86_64" ;;
        aarch64|arm64) arch_part="aarch64" ;;
        *) echo "homn install: unsupported arch '$arch' — build from source: cargo install --git https://github.com/$REPO homn-bin" >&2; exit 1 ;;
    esac
    echo "${arch_part}-${os_part}"
}
TRIPLE=$(detect_triple)

# --- resolve the release tag ------------------------------------------------
if [ -z "$VERSION" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)
    [ -n "$VERSION" ] || { echo "homn install: could not resolve the latest release" >&2; exit 1; }
fi

ASSET="homn-${VERSION}-${TRIPLE}.tar.gz"
BASE="https://github.com/$REPO/releases/download/$VERSION"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "homn install: downloading $ASSET ($VERSION)"
curl -fsSL "$BASE/$ASSET"        -o "$TMP/$ASSET"
curl -fsSL "$BASE/$ASSET.sha256" -o "$TMP/$ASSET.sha256"

# --- verify checksum --------------------------------------------------------
echo "homn install: verifying checksum"
( cd "$TMP" && \
  if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$ASSET.sha256"; \
  else shasum -a 256 -c "$ASSET.sha256"; fi ) \
  || { echo "homn install: CHECKSUM MISMATCH — aborting" >&2; exit 1; }

# --- install ----------------------------------------------------------------
# The release tarball wraps everything in a top-level homn-<tag>-<triple>/ dir;
# --strip-components=1 drops it so the binary lands directly at $TMP/homn.
tar -xzf "$TMP/$ASSET" -C "$TMP" --strip-components=1
mkdir -p "$BIN_DIR"
mv "$TMP/homn" "$BIN_DIR/homn"
chmod +x "$BIN_DIR/homn"
echo "homn install: installed to $BIN_DIR/homn"

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo ""
       echo "  NOTE: $BIN_DIR is not on your PATH. Add this to your shell rc file:"
       echo "      export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

echo ""
echo "homn installed. Next: run  homn setup"
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x install.sh`

- [ ] **Step 3: Verify with shellcheck**

Run: `shellcheck install.sh`
Expected: no findings. (If `shellcheck` is not installed: `sudo pacman -S shellcheck` on Arch/CachyOS.)

- [ ] **Step 4: Syntax-check the script**

Run: `sh -n install.sh`
Expected: no output, exit 0 — the script parses as valid POSIX `sh` without executing it.
(Full behaviour is verified post-launch by the smoke test in "Final verification" once a release exists.)

- [ ] **Step 5: Commit**

```bash
git add install.sh
git commit -m "feat(install): curl|sh installer with checksum verification"
```

---

## Task 9: `shellcheck` CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Read the current workflow**

Run: `cat .github/workflows/ci.yml`
Note the existing job names and the `jobs:` indentation.

- [ ] **Step 2: Add a shellcheck job**

In `.github/workflows/ci.yml`, under `jobs:`, add a new job (sibling to the existing jobs, same indentation):

```yaml
  shellcheck:
    name: shellcheck install.sh
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run shellcheck
        run: shellcheck install.sh
```

- [ ] **Step 3: Validate the YAML**

Run: `python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/ci.yml")); print("ci.yml is valid YAML")'`
Expected: `ci.yml is valid YAML`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: shellcheck install.sh"
```

---

## Task 10: Release workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/release.yml` with this content:

```yaml
name: release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  build:
    name: build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-24.04-arm
          - target: x86_64-apple-darwin
            os: macos-latest
          - target: aarch64-apple-darwin
            os: macos-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust target
        run: rustup target add ${{ matrix.target }}
      - name: Build
        run: cargo build --release --locked --target ${{ matrix.target }} -p homn-bin
      - name: Package
        run: |
          TAG="${GITHUB_REF_NAME}"
          STAGE="homn-${TAG}-${{ matrix.target }}"
          mkdir "$STAGE"
          cp "target/${{ matrix.target }}/release/homn" "$STAGE/"
          strip "$STAGE/homn" || true
          cp LICENSE README.md "$STAGE/"
          cp -r policies "$STAGE/"
          tar -czf "${STAGE}.tar.gz" "$STAGE"
          if command -v sha256sum >/dev/null 2>&1; then
            sha256sum "${STAGE}.tar.gz" > "${STAGE}.tar.gz.sha256"
          else
            shasum -a 256 "${STAGE}.tar.gz" > "${STAGE}.tar.gz.sha256"
          fi
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: homn-${{ matrix.target }}
          path: |
            homn-*.tar.gz
            homn-*.tar.gz.sha256

  release:
    name: publish release
    needs: build
    runs-on: ubuntu-latest
    steps:
      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          merge-multiple: true
          path: dist
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            dist/homn-*.tar.gz
            dist/homn-*.tar.gz.sha256
```

- [ ] **Step 2: Validate the YAML**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/release.yml")); print("release.yml is valid YAML")'`
Expected: `release.yml is valid YAML`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: release workflow — cross-compile + publish binaries on tag"
```

---

## Task 11: Docs — two-line quick-start

**Files:**
- Modify: `README.md`
- Modify: `docs/getting-started.md`

- [ ] **Step 1: Update the README quick-start**

In `README.md`, find the install section (the block containing `homn install --apply` and the `cat > ~/.config/homn/policies/default.rhai` heredoc). Replace that block with:

```markdown
## Install

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

`homn setup` seeds a policy, installs the Claude Code hook (with a backup), and starts
the background daemon. It is idempotent — safe to re-run. Reverse it any time with
`homn uninstall`.

<details>
<summary>Or, step by step / build from source</summary>

```sh
cargo install --git https://github.com/rohansx/homn homn-bin
homn rule edit          # seed + edit your policy
homn install --apply    # install the Claude Code hook
homn daemon             # or install a service unit — see docs/getting-started.md
```
</details>
```

- [ ] **Step 2: Update getting-started.md**

In `docs/getting-started.md`, add immediately after the first heading a new section:

```markdown
## Quick start

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

That installs the binary, seeds a policy, wires the Claude Code hook, and starts the
daemon. The rest of this page covers the manual, step-by-step path and how each piece works.
```

- [ ] **Step 3: Verify the links resolve**

Run: `grep -n "install.sh\|homn setup\|homn uninstall" README.md docs/getting-started.md`
Expected: the new commands appear in both files.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/getting-started.md
git commit -m "docs: two-line quick-start (curl | sh + homn setup)"
```

---

## Final verification

- [ ] **Step 1: Full workspace gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all tests pass, no clippy warnings, formatting clean.

- [ ] **Step 2: End-to-end setup smoke test (sandboxed)**

Run:
```bash
rm -rf /tmp/homn-setup-test && mkdir -p /tmp/homn-setup-test/{config,data,run}
XDG_CONFIG_HOME=/tmp/homn-setup-test/config \
XDG_DATA_HOME=/tmp/homn-setup-test/data \
XDG_RUNTIME_DIR=/tmp/homn-setup-test/run \
  ./target/debug/homn setup --no-service
```
Expected: report shows `policy: seeded …`, `hook: installed …`, `service: skipped (--no-service)`; `/tmp/homn-setup-test/config/homn/policies/default.rhai` exists and parses.

- [ ] **Step 3: Commit any formatting fixes**

```bash
cargo fmt --all
git diff --quiet || git commit -am "style: cargo fmt"
```

---

## Notes for the executor

- **homn-hook is TDD-mandatory** (Constitution VI). Tasks 1–6 follow red-green strictly: write the test, watch it fail, implement, watch it pass.
- The `systemctl` / `launchctl` wrappers (Task 4) are thin plumbing — not unit-tested; they are exercised by the Task 11 final smoke test and real use.
- `install.sh` and `release.yml` cannot be fully tested until the first `v*` tag is pushed and a release exists. The post-release CI check for `install.sh` is deferred to that point.
- Commit cadence above assumes the user's git workflow allows commits; if working on `master`, branch first per the repo's conventions.
