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
}
