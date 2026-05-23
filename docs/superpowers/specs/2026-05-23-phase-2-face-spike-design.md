# Design: Phase 2 spike — Tauri window opens with the idle face

**Date:** 2026-05-23
**Status:** approved (brainstorming → implementation)
**Phase 2 context:** [`docs/phases/phase-2-face.md`](../../phases/phase-2-face.md) (weeks 5–10). This spike is the *risk-buying first slice* — it precedes any of the documented weeks-5/6 work.

## Goal

Prove on this machine that **Tauri 2 + KDE Wayland** can produce a small transparent
always-on-top frameless window. One question, one answer, two paths:

- **It works** → next slice adds character states + event-bus subscription.
- **It doesn't** → we learn why now, before building four other things on top.

Nothing else is in scope. No animations, no events, no states, no UI chrome.

## Non-goals (YAGNI)

- Multiple character states (one face only: `◕ ◡ ◕`).
- Event-bus subscription to the daemon (next slice).
- Hover panel, right-click menu, mute mode (week 7–8 in the roadmap).
- Svelte / Vite / any frontend framework — plain HTML for the spike.
- Production bundling (`tauri build`), `.deb` / `.app` packaging.
- X11 fallback — we are validating KDE Wayland specifically.
- Multi-monitor, position persistence, themes (week 9–10).
- `homn face` CLI subcommand.

Each becomes its own future slice with its own spec.

## Deliverable

A new workspace member `crates/homn-face/`. Single binary, single static frontend,
**no new external tools required**:

```
crates/homn-face/
├── Cargo.toml          # tauri + tauri-build deps
├── tauri.conf.json     # window: 200x120, transparent, alwaysOnTop, frameless
├── build.rs            # one line: tauri_build::build()
├── src/main.rs         # ~10 lines: tauri::Builder::default().run(generate_context!())
├── dist/
│   └── index.html      # static frontend with the idle face
└── tests/
    └── config.rs       # asserts tauri.conf.json parses
```

Prereqs are already on the user's machine (`webkit2gtk-4.1`, Node, Cargo). We do **not**
install `cargo install tauri-cli` for the spike — `cargo run -p homn-face` is enough for
dev; the CLI only matters for production bundling, which is out of scope.

### Window properties

Sourced from [`docs/architecture/face.md`](../../architecture/face.md):

| Property        | Value     | Why                                          |
|-----------------|-----------|----------------------------------------------|
| `width`         | `200`     | small, peripheral                            |
| `height`        | `120`     | "                                            |
| `transparent`   | `true`    | character floats, no background card         |
| `alwaysOnTop`   | `true`    | the entire point of the face                 |
| `decorations`   | `false`   | frameless (no titlebar/borders)              |
| `resizable`     | `false`   | fixed size for v0                            |
| `skipTaskbar`   | `true`    | don't clutter the KDE taskbar                |
| `title`         | `"homn"`  | identifies the process                       |

### Frontend

Single static `dist/index.html`: transparent `<body>`, centered `<pre>` containing
`◕ ◡ ◕`, white monospace, no JavaScript. Tauri loads it via `frontendDist: "dist"`.

### Workspace integration

Add `"crates/homn-face"` to the workspace `Cargo.toml` `members` array. Nothing else
in the workspace changes — no daemon dependency, no `homn-bin` change, no `homn install`
change. The face is **default OFF** (Constitution V) — for the spike it's run manually
via `cargo run -p homn-face`. A `homn face --enable` subcommand lands later.

## Acceptance criteria

Running `cargo run -p homn-face` on the developer's machine must produce a window where:

1. ✅ A window opens — confirms Tauri + webview2gtk-4.1 is functional on this host.
2. ✅ No titlebar / no borders — confirms `decorations: false` is honoured.
3. ✅ The background is transparent (desktop visible through it) — confirms KWin
   accepts the compositor alpha hint.
4. ✅ The window stays on top of other windows — confirms KWin honours `alwaysOnTop`
   on Wayland (the risks doc flags this; we observe what KWin actually does).
5. ✅ The character `◕ ◡ ◕` is visible and centered.

Any of (3), (4), or (5) failing is *useful data* — the spike has done its job either
way. The next slice's design depends on what we learn here.

## Testing strategy

The runtime check above is the primary validation; we add minimal automated guards:

- **Config-parse test** (`crates/homn-face/tests/config.rs`): reads
  `tauri.conf.json` and asserts it's valid JSON with the expected window keys
  (`width`, `height`, `transparent`, `alwaysOnTop`, `decorations`). Catches a
  malformed config in CI.
- **CI compile job**: `cargo build -p homn-face` is added to `.github/workflows/ci.yml`
  so the crate compiles on every PR. **No GUI runtime in CI** — webkit2gtk in headless
  CI is more trouble than it's worth for a spike; the manual run on the dev's machine
  is the acceptance test.

The Tauri scaffold itself is configuration code, not policy/audit/hook code — TDD per
Constitution VI does not apply. The config-parse test is a regression guard.

## Architecture & boundaries

| Unit                        | Responsibility                                       | Depends on                |
|-----------------------------|------------------------------------------------------|---------------------------|
| `crates/homn-face/`         | Tauri scaffold: open a window, load the static frontend | `tauri`, `tauri-build`, `serde`, `serde_json` (test) |
| `crates/homn-face/dist/index.html` | Static character render                       | none                      |
| `crates/homn-face/tauri.conf.json` | Window configuration                          | none                      |

`homn-face` is intentionally a leaf crate: no other workspace crate depends on it,
and it depends on no other workspace crate. This keeps the spike fully isolable —
if Tauri's deps don't compile, the rest of the workspace is unaffected.

## Constitution check

- **Local-first (I)**: no network. Frontend is a static file packaged with the binary.
- **Conservative defaults (V)**: face is opt-in. Spike does not touch `homn install`,
  `~/.claude/settings.json`, the running daemon, or any other surface. Nothing
  auto-starts.
- **Tests-first (VI)**: not applicable to Tauri scaffold code; config-parse guard
  test is added as a regression guard.
- **Audit (III)**: unaffected — the face is read-only and does not write decisions.

## Resolved decisions

- **Crate directory**: `crates/homn-face/` (consistent with `docs/architecture/overview.md`).
- **Frontend stack**: plain HTML, no framework.
- **Tauri CLI**: not installed for the spike (`cargo run` is enough for dev).
- **Character**: `◕ ◡ ◕` (idle, per face docs).
- **Compositor target**: KDE Wayland (KWin) only for this spike.

## What we learn (next-slice inputs)

The spike's output is *information*, not just a binary. Specifically, we learn:

1. Whether Tauri 2 + webkit2gtk-4.1 builds + launches cleanly on this Arch host.
2. Whether KWin Wayland honours `transparent: true`.
3. Whether KWin Wayland honours `alwaysOnTop: true`.
4. Whether `decorations: false` gives a clean frameless window.

The next slice (states + event-bus) is gated on (1) at minimum; (2)–(4) inform
whether we need an X11 fallback path or to accept a non-floating window on KWin.
