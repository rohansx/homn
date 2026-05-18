//! `homn setup` / `homn uninstall` orchestration.
//!
//! `setup` takes a fresh install from "binary on disk" to "daemon running, hook wired,
//! policy seeded" in one idempotent command; `uninstall` reverses it. The pure,
//! side-effect-free pieces (policy seeding, service-file generation, hook removal) live
//! here and are unit-tested; the thin `systemctl` / `launchctl` calls are isolated.

use std::path::{Path, PathBuf};

use crate::install::{backup_path_for, remove_homn_entry, run_install};
use crate::InstallReport;

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

/// What [`run_uninstall`] did, for the CLI to report.
pub struct UninstallReport {
    /// Whether homn's hook entry was removed from `settings.json`.
    pub hook_removed: bool,
    /// Whether the background service was stopped + removed.
    pub service_removed: bool,
}

/// Run `homn uninstall`: remove the hook, and (if `remove_service`) the service.
/// Policy files and the audit DB are left in place — that is the user's data.
pub fn run_uninstall(
    settings_path: &Path,
    remove_service: bool,
) -> anyhow::Result<UninstallReport> {
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
            // Mirror run_install --apply: back up the original, then write atomically via
            // a temp file + rename so a crash mid-write cannot truncate the user's file.
            let backup = backup_path_for(settings_path);
            std::fs::copy(settings_path, &backup)?;
            let tmp = settings_path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string_pretty(&settings)?)?;
            std::fs::rename(&tmp, settings_path)?;
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
        assert!(
            !report.service_removed,
            "remove_service=false skips the service"
        );

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(!crate::install::is_homn_entry_present(&after));

        // A timestamped backup of the original settings.json must exist alongside the updated file.
        let backup_exists = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.starts_with("settings.json.bak.")
            });
        assert!(
            backup_exists,
            "run_uninstall must write a timestamped backup"
        );
    }

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
}
