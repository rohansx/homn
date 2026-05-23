# Phase 2 face spike — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scaffold a new `crates/homn-face/` Tauri 2 crate so that `cargo run -p homn-face` opens a small transparent always-on-top frameless window with the idle face `◕ ◡ ◕` on the developer's machine.

**Architecture:** A single leaf crate, no workspace dependencies, no frontend framework. Plain static HTML loaded by Tauri 2; no `tauri-cli` install needed. The Tauri scaffold is configuration code — Tasks 1, 3, 4 are scaffold; Task 2 is a config-parse regression guard test (not TDD-mandated, this isn't policy/audit/hook code).

**Tech Stack:** Tauri 2.x, webkit2gtk-4.1 (Linux), plain HTML, serde_json (test only).

**Spec:** [`docs/superpowers/specs/2026-05-23-phase-2-face-spike-design.md`](../specs/2026-05-23-phase-2-face-spike-design.md)

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` (workspace) | Modify | Add `crates/homn-face` to `members` and `homn-face` / `tauri` / `tauri-build` to `workspace.dependencies` |
| `crates/homn-face/Cargo.toml` | Create | Crate manifest — `tauri` runtime dep, `tauri-build` build-dep, `serde_json` dev-dep |
| `crates/homn-face/build.rs` | Create | One-liner — `tauri_build::build()` |
| `crates/homn-face/tauri.conf.json` | Create | Window config: 200×120, transparent, alwaysOnTop, frameless, skipTaskbar |
| `crates/homn-face/src/main.rs` | Create | Tauri runtime entry — `Builder::default().run(generate_context!())` |
| `crates/homn-face/dist/index.html` | Create | Static frontend — transparent body, centered `<pre>` with `◕ ◡ ◕` |
| `crates/homn-face/icons/icon.png` | Create | Tauri 2's `generate_context!()` hard-requires an RGBA `icons/icon.png` at compile time. A minimal 32×32 transparent RGBA PNG is sufficient for the spike. |
| `crates/homn-face/tests/config.rs` | Create | Regression guard — assert `tauri.conf.json` parses with the expected window keys |
| `.gitignore` | Modify | Add `crates/homn-face/gen/` — `tauri-build` regenerates JSON schemas on every build; tracking them creates 4500 lines of build-output noise and merge conflicts on regen. |
| `.github/workflows/ci.yml` | Modify | Add a `face-build` job that apt-installs webkit2gtk-4.1 and runs `cargo build -p homn-face` |
| `docs/architecture/face.md` | Modify | Append a "Spike results" subsection with the observed outcomes against the 5 acceptance criteria |

---

## Task 1: Scaffold the `homn-face` crate

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/homn-face/Cargo.toml`
- Create: `crates/homn-face/build.rs`
- Create: `crates/homn-face/tauri.conf.json`
- Create: `crates/homn-face/src/main.rs`
- Create: `crates/homn-face/dist/index.html`

- [ ] **Step 1: Add `homn-face` to the workspace**

In the repo-root `Cargo.toml`, find the `[workspace]` `members = [...]` list and add `"crates/homn-face",` to it (alphabetical position, between `crates/homn-daemon` and `crates/homn-hook`). Then in `[workspace.dependencies]`, after the existing `homn-*` path lines, add:

```toml
homn-face      = { path = "crates/homn-face", version = "0.1.0" }
```

And after the `homn-*` block, add the Tauri deps so the version is pinned in one place (use `features = []` — Tauri 2's default features include the `wry` webview backend, which we DO want; `default-features = false` would strip it and break the build):

```toml
tauri          = { version = "2", features = [] }
tauri-build    = { version = "2", features = [] }
```

- [ ] **Step 2: Create the crate manifest**

Create `crates/homn-face/Cargo.toml`:

```toml
[package]
name          = "homn-face"
version       = { workspace = true }
edition       = { workspace = true }
rust-version  = { workspace = true }
authors       = { workspace = true }
license       = { workspace = true }
repository    = { workspace = true }
description   = "Tauri-backed always-on-top ASCII face window for homn (Phase 2)."

[build-dependencies]
tauri-build   = { workspace = true }

[dependencies]
tauri         = { workspace = true }

[dev-dependencies]
serde_json    = { workspace = true }
```

- [ ] **Step 3: Create the Tauri build script**

Create `crates/homn-face/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 4: Create the Tauri config**

Create `crates/homn-face/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "homn-face",
  "version": "0.1.0",
  "identifier": "sh.homn.face",
  "build": {
    "frontendDist": "dist"
  },
  "app": {
    "windows": [
      {
        "title": "homn",
        "width": 200,
        "height": 120,
        "transparent": true,
        "alwaysOnTop": true,
        "decorations": false,
        "resizable": false,
        "skipTaskbar": true
      }
    ],
    "security": {
      "csp": null
    }
  }
}
```

- [ ] **Step 5: Create the runtime entry point**

Create `crates/homn-face/src/main.rs`:

```rust
//! homn-face — Phase 2 spike.
//!
//! Opens a small transparent always-on-top frameless window that renders the idle
//! face `◕ ◡ ◕`. No event-bus wiring yet; the spike's sole job is to validate that
//! Tauri 2 + webkit2gtk-4.1 + KWin Wayland produce a usable window on this host.
//! See `docs/superpowers/specs/2026-05-23-phase-2-face-spike-design.md`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running the homn-face Tauri application");
}
```

- [ ] **Step 6: Create the static frontend**

Create `crates/homn-face/dist/index.html`:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>homn</title>
  <style>
    html, body {
      margin: 0;
      padding: 0;
      width: 100vw;
      height: 100vh;
      background: transparent;
      overflow: hidden;
    }
    body {
      display: grid;
      place-items: center;
      font-family: ui-monospace, "JetBrains Mono", "Fira Code", Menlo, monospace;
      color: #ffffff;
      text-shadow: 0 1px 2px rgba(0, 0, 0, 0.45);
      user-select: none;
    }
    pre {
      margin: 0;
      font-size: 28px;
      line-height: 1;
      letter-spacing: 2px;
    }
  </style>
</head>
<body>
  <pre>◕ ◡ ◕</pre>
</body>
</html>
```

- [ ] **Step 7: Confirm the workspace builds**

Run: `cargo build -p homn-face`

Expected: a successful build (compiles `tauri`, `tauri-build`, `homn-face`; binary at `target/debug/homn-face`). The first build pulls a lot of Tauri deps and may take 1–3 minutes.

If it fails with a missing-system-library error (e.g. `webkit2gtk-4.1 not found`), the host is missing the webview headers — install them per the README of `webkit2gtk` on the platform. On Arch / CachyOS the package is already `webkit2gtk-4.1`.

- [ ] **Step 8: Confirm the workspace test suite still passes**

Run: `cargo test --workspace`

Expected: all existing tests pass (142 before this change; same count after, since no behavior tests added yet).

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/homn-face/
git commit -m "feat(face): scaffold homn-face Tauri 2 crate (Phase 2 spike)" \
  -m "Adds a leaf crate that opens a 200x120 transparent always-on-top frameless window rendering the idle face ◕ ◡ ◕ via plain HTML. No event-bus wiring yet — this slice validates Tauri 2 + KWin Wayland on the developer's machine. See docs/superpowers/specs/2026-05-23-phase-2-face-spike-design.md."
```

---

## Task 2: Config-parse regression guard test

**Files:**
- Create: `crates/homn-face/tests/config.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/homn-face/tests/config.rs`:

```rust
//! Regression guard: tauri.conf.json must parse and carry every window key the
//! face spike depends on (transparent, alwaysOnTop, decorations, skipTaskbar, etc.).
//! If a future edit drops one of these keys silently, this test fails on the next CI run.

use serde_json::Value;

#[test]
fn tauri_conf_json_parses_with_expected_window_keys() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let raw = std::fs::read_to_string(path).expect("tauri.conf.json is readable");
    let v: Value = serde_json::from_str(&raw).expect("tauri.conf.json is valid JSON");

    let win = &v["app"]["windows"][0];
    for key in [
        "title",
        "width",
        "height",
        "transparent",
        "alwaysOnTop",
        "decorations",
        "resizable",
        "skipTaskbar",
    ] {
        assert!(
            !win[key].is_null(),
            "tauri.conf.json window[0] is missing the required key `{key}`"
        );
    }

    assert_eq!(win["transparent"], Value::Bool(true), "transparent must be true");
    assert_eq!(win["alwaysOnTop"], Value::Bool(true), "alwaysOnTop must be true");
    assert_eq!(win["decorations"], Value::Bool(false), "decorations must be false");
    assert_eq!(win["width"], Value::from(200), "width must be 200");
    assert_eq!(win["height"], Value::from(120), "height must be 120");
}
```

- [ ] **Step 2: Run the test, confirm it passes**

Run: `cargo test -p homn-face --test config`

Expected: `test result: ok. 1 passed; 0 failed`.

(Note: this is a regression guard, not strict TDD — the Tauri scaffold is configuration code, not policy/audit/hook code, so Constitution VI's tests-first mandate does not apply. The test exists to catch a future drift in `tauri.conf.json`.)

- [ ] **Step 3: Verify the test fails when a key is removed (manual sanity check)**

Temporarily delete the line `"skipTaskbar": true` from `crates/homn-face/tauri.conf.json` and run the test:

```sh
cargo test -p homn-face --test config 2>&1 | grep -E "FAIL|missing"
```

Expected: the test fails with `tauri.conf.json window[0] is missing the required key \`skipTaskbar\``.

Restore the line. Re-run the test to confirm green.

- [ ] **Step 4: Commit**

```bash
git add crates/homn-face/tests/config.rs
git commit -m "test(face): config-parse regression guard for tauri.conf.json"
```

---

## Task 3: CI build job for `homn-face`

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Inspect the existing workflow**

Run: `grep -n "^  [a-z]*:" .github/workflows/ci.yml`

This lists the current jobs (`fmt`, `clippy`, `test`, `shellcheck`, `msrv`). You'll add `face-build` as a sibling.

- [ ] **Step 2: Add the `face-build` job**

In `.github/workflows/ci.yml`, find the `shellcheck` job and add the following job immediately after it (same indentation level):

```yaml
  face-build:
    name: build homn-face (Tauri 2)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Tauri 2 system deps
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libgtk-3-dev \
            libsoup-3.0-dev \
            libjavascriptcoregtk-4.1-dev
      - uses: dtolnay/rust-toolchain@stable
      - name: Build homn-face
        run: cargo build -p homn-face --locked
      - name: Config-parse test
        run: cargo test -p homn-face --test config --locked
```

This pins the Rust toolchain (parity with the other jobs), installs the Tauri 2 Linux system libs (`webkit2gtk-4.1` + GTK + libsoup-3 + JavaScriptCore), then builds + runs the config test. The job is GUI-less — it does not launch the window (webkit2gtk in headless CI is fragile and out of scope for the spike).

- [ ] **Step 3: Validate the YAML**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml")); print("ci.yml: valid YAML")'`

Expected: `ci.yml: valid YAML`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(face): build homn-face + run config test on every PR" \
  -m "Installs the Tauri 2 Linux system deps (webkit2gtk-4.1, gtk-3, libsoup-3, javascriptcoregtk-4.1), pins the Rust toolchain, builds homn-face with --locked, and runs the config-parse regression test. No GUI runtime in CI — headless webkit2gtk is fragile."
```

---

## Task 4: Manual acceptance run

**Files:**
- Modify: `docs/architecture/face.md`

This task is the spike's actual point: run the binary and observe whether Tauri 2 + KWin Wayland delivers each acceptance criterion. The outcome (whichever way it goes) is recorded in the architecture doc as a "Spike results" section so the next slice has a starting point.

- [ ] **Step 1: Run the face**

Run: `cargo run -p homn-face`

A window should appear. Leave it open while you complete the checks below.

- [ ] **Step 2: Observe the five acceptance criteria**

For each, record `pass` / `fail` / `partial` with one line of detail:

1. ✅ **Window opens** — does any window appear at all? (Confirms Tauri + webkit2gtk-4.1 works on this host.)
2. ✅ **Frameless** — is there a titlebar / border? (Should be neither.)
3. ✅ **Transparent** — can you see the desktop through the window background? (Should be yes.)
4. ✅ **Always-on-top** — open another window over it (e.g. a terminal), then click that other window. Does the face window stay visible? (KWin Wayland may or may not honour this — the risks doc flags it.)
5. ✅ **Character visible** — is `◕ ◡ ◕` rendered, white, centered?

If the window appears in a weird position, ignore that for now — position handling is a later slice.

- [ ] **Step 3: Capture evidence**

If `grim` is installed (`command -v grim`), take a screenshot for posterity:

```sh
mkdir -p docs/launch/phase-2
grim -t png docs/launch/phase-2/face-spike.png
ls -la docs/launch/phase-2/face-spike.png
```

If `grim` is absent, skip; the written observations are the canonical record.

- [ ] **Step 4: Close the window**

Press `Ctrl+C` in the terminal running `cargo run -p homn-face`. The window closes.

(Wayland-on-KDE often does not give frameless windows a close affordance — `Ctrl+C` on the parent terminal is the spike's intended shutdown.)

- [ ] **Step 5: Record the spike results in the architecture doc**

In `docs/architecture/face.md`, append a new section at the end of the file:

```markdown

## Spike results — 2026-05-23 (Phase 2 v0)

The first slice (see `docs/superpowers/specs/2026-05-23-phase-2-face-spike-design.md`)
ran on a CachyOS + KDE Plasma 6 Wayland host with `webkit2gtk-4.1`.

| Acceptance criterion       | Outcome             | Notes                                  |
|----------------------------|---------------------|----------------------------------------|
| Window opens               | <pass/fail>         | <one line>                             |
| Frameless                  | <pass/fail/partial> | <one line>                             |
| Transparent                | <pass/fail/partial> | <one line>                             |
| Always-on-top (KWin)       | <pass/fail/partial> | <one line>                             |
| Character `◕ ◡ ◕` visible | <pass/fail>         | <one line>                             |

**Next slice depends on:** if always-on-top is `partial` or `fail` on KWin, the next
slice adds a "best-effort" path that uses the standard `setAlwaysOnTop` hint plus a
documented fallback (notify-only mode) rather than a hard requirement. If transparent
is `partial`, document the platform-specific opacity workaround.

If the optional `grim` screenshot was captured, it lives at
`docs/launch/phase-2/face-spike.png`.
```

Replace each `<pass/fail>` and `<one line>` with the actual observations from Step 2.

- [ ] **Step 6: Commit**

```bash
git add docs/architecture/face.md
# include the screenshot only if grim captured one
[ -f docs/launch/phase-2/face-spike.png ] && git add docs/launch/phase-2/face-spike.png
git commit -m "docs(face): record Phase 2 spike results on KDE Wayland"
```

---

## Final verification

- [ ] **Step 1: Workspace gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`

Expected: all tests pass (142 from before + 1 new config test = 143), no clippy warnings, formatting clean.

- [ ] **Step 2: Build the new face crate cleanly**

Run: `cargo build -p homn-face --locked`

Expected: clean build, no warnings.

- [ ] **Step 3: Confirm CI workflow validity**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml")); print("ci.yml: valid YAML")'`

Expected: `ci.yml: valid YAML`.

---

## Notes for the executor

- **Tauri 2's first build is slow.** It pulls a lot of crates (`wry`, `tao`, GTK / WebKit bindings). 1–3 minutes is normal on a warm cache; the very first cold build on a fresh checkout can be 5+ minutes.
- **`tauri-cli` is not installed.** It is **not required** for `cargo run -p homn-face`; we use it only for production bundling, which is out of scope for this spike. If a step says "install tauri-cli," stop — that's a plan defect.
- **No event-bus wiring.** `homn-face` does not connect to the homn daemon in this slice. That is the next slice's job.
- **KDE Wayland always-on-top is documented as risky** in `docs/risks/known-unknowns.md`. The Task 4 observations are the source of truth for what KWin actually does.
- **Commit attribution is disabled globally** (`~/.claude/settings.json`) — do NOT add any `Co-Authored-By` trailer to the commits in this plan.
