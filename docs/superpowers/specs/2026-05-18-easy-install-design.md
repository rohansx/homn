# Design: easy install & setup for homn

**Date:** 2026-05-18
**Status:** approved (brainstorming → implementation)
**Relates to:** Phase 1 Polish — extends T080 (`homn install`) and T081 (service unit).

## Problem

Installing homn today means: have Rust, clone the repo, `cargo install`, hand-copy a
policy, run `homn install`, hand-write a systemd unit, start it. Six steps and a toolchain.
A manual walk-through on 2026-05-18 also exposed sharp edges — a `cp -i` alias silently
skipped a file, and a broken policy crash-looped the daemon with no guard.

**Goal:** a new user runs two commands and homn is guarding their Claude Code sessions:

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

## Non-goals (YAGNI)

Package managers (AUR, Homebrew), Windows, an interactive wizard, a custom domain. All
can come later; none block the launch.

## Deliverables

### 1. `install.sh` — `curl | sh` installer (repo root)

Bash, `set -eu`, POSIX-portable. Behaviour:

1. Detect platform: `uname -s` → `Linux`/`Darwin`, `uname -m` → `x86_64`/`aarch64`|`arm64`.
   Map to a Rust target triple (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
   `x86_64-apple-darwin`, `aarch64-apple-darwin`).
2. Resolve the release: GitHub API `releases/latest` → `tag_name`, or `--version vX.Y.Z`.
3. Download `homn-<tag>-<triple>.tar.gz` + its `.sha256`; **verify the checksum**
   (`sha256sum -c` / `shasum -a 256 -c`). Abort on mismatch.
4. Extract `homn` to `${HOMN_BIN_DIR:-$HOME/.local/bin}/homn`, `chmod +x`. No sudo.
5. If the bin dir is not on `$PATH`, print the exact `export PATH=...` line to add.
6. Print: `homn installed — next: run \`homn setup\``.

Flags: `--version`, `--bin-dir`. Failure modes are explicit: no `curl`/`tar` → name the
missing tool; unsupported platform → "build from source: `cargo install --git ...`";
checksum mismatch → abort without installing.

### 2. `.github/workflows/release.yml` — release CI

Trigger: push of a `v*` tag. Matrix over the four target triples (Linux x86_64/aarch64,
macOS x86_64/aarch64). Per target:

- `cargo build --release --locked --target <triple> -p homn-bin`, then `strip`.
- Package `homn-<tag>-<triple>.tar.gz` containing `homn`, `LICENSE`, `README.md`,
  and the `policies/` samples; generate a `.sha256` sidecar.

A final job creates the GitHub Release for the tag and attaches all tarballs + checksums.
Linux aarch64 builds on an ARM runner (or `cross`); macOS targets build on `macos-latest`.

`shellcheck` for `install.sh` is added to the existing CI workflow (`ci.yml`), not here.

### 3. `homn setup` & `homn uninstall` — first-run commands

New `Setup` and `Uninstall` subcommands in `homn-bin`. Orchestration logic lives in a new
**`homn-hook/src/setup.rs`** module (homn-hook already owns hook install + PTY; it is the
integration-surface crate). Constitution VI makes homn-hook TDD-mandatory — the setup
logic is written tests-first.

**`homn setup [--no-service] [--policy default|strict|relaxed]`** — idempotent; each step
reports status:

1. **Policy** — target `~/.config/homn/policies/default.rhai`. Absent → write the bundled
   profile (`include_str!`-baked, default unless `--policy` says otherwise). Present →
   parse it; if it parses, leave it untouched; if not, **warn loudly and do not clobber**
   (this is the broken-policy case from the manual run).
2. **Hook** — reuse `homn_hook::run_install(settings_path, apply=true, …)` — merges the
   PermissionRequest hook into `~/.claude/settings.json` with a timestamped backup.
3. **Service** (skipped under `--no-service`) — detect the init system:
   - Linux + systemd → write `~/.config/systemd/user/homn.service` from a template with
     `ExecStart` set to the binary's **resolved absolute path** (`std::env::current_exe()`,
     so it is correct whether homn lives in `~/.local/bin` or `~/.cargo/bin`); then
     `systemctl --user daemon-reload` and `enable --now`.
   - macOS → write `~/Library/LaunchAgents/sh.homn.daemon.plist`; `launchctl load`.
   - Anything else → print copy-paste manual instructions.
4. **Verify** — poll for the daemon socket for a few seconds; report running / not-running.

Final summary: policy path, hook status, service status, and `homn rule edit` as the
next step. Re-running `homn setup` is always safe.

**`homn uninstall [--purge]`** — reverses setup: stop + disable + remove the service unit;
remove homn's entry from `settings.json` (leaving the user's other hooks intact). Policy
and audit DB are **kept** by default, with a printed note of their paths; `--purge` also
removes `~/.config/homn` and `~/.local/share/homn`.

`systemd_unit(exec_path)` keeps `dist/homn.service` as the single source of truth: it
`include_str!`s that committed file and rewrites the one `ExecStart` line to the resolved
absolute path. The committed `dist/homn.service` keeps `%h/.cargo/bin/homn` — the correct
default for `cargo install`-based manual installs. macOS has no committed plist; the
launchd plist is generated by a `format!` template in `launchd_plist(exec_path)`.

### 4. Docs

`README.md` and `docs/getting-started.md` lead with the two-line quick-start; the existing
manual steps are kept below as "or, step by step".

## Architecture & boundaries

| Unit | Responsibility | Depends on |
|------|----------------|------------|
| `install.sh` | platform detect → download → verify → place binary | GitHub Releases, `curl`, `tar`, `sha256sum` |
| `release.yml` | cross-compile + package + publish on tag | GitHub Actions |
| `homn-hook/src/setup.rs` | seed policy, generate service unit, orchestrate, uninstall | `homn-hook::install`, `homn-policy` (parse check) |
| `homn-bin` `Setup`/`Uninstall` arms | thin CLI wrappers + status output | `homn-hook::setup` |

The side-effecting calls (`systemctl`, `launchctl`) are isolated behind a small
`ServiceManager` abstraction so unit-file *generation* and orchestration are tested
without touching the real system.

## Testing

- **`install.sh`** — `shellcheck` in CI plus a `sh -n` syntax check; full behaviour
  (platform detection, download, checksum) is verified by the post-release smoke test
  that runs the script against real release assets.
- **`homn-hook/src/setup.rs`** (TDD, tests-first):
  - `seed_policy`: writes when absent; skips (no-op) when a parseable policy exists;
    reports "present but unparseable" without overwriting.
  - `systemd_unit(exec_path)` / `launchd_plist(exec_path)`: pure generators — assert the
    output embeds the given absolute path and the expected directives.
  - idempotency: running the orchestration twice leaves the same end state.
  - uninstall: removing homn's hook entry preserves other PermissionRequest hooks.
- **`release.yml`** — a post-release CI job runs `install.sh` against the freshly published
  assets (active once the first `v*` tag exists).

## Constitution check

- **Local-first (I):** the installer pulls from GitHub Releases; setup touches only local
  files and the local service manager. No homn-operated network service.
- **Conservative defaults (V):** setup is idempotent, never clobbers an existing policy,
  backs up `settings.json`, and offers `--no-service`. `uninstall` keeps user data unless
  `--purge`.
- **Tests-first (VI):** `homn-hook` is TDD-mandatory; `setup.rs` is written tests-first.
- **Audit (III):** unaffected — setup does not change the decision/audit path.

## Resolved decisions

- **`homn uninstall`:** included — the manual run showed a clean reversal is needed.
- **Install location:** `install.sh` → `~/.local/bin` (no sudo). `homn setup` writes the
  service unit with `current_exe()`, so it is correct for any install location.
