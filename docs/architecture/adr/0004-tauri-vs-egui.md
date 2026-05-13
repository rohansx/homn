# ADR-0004 — Tauri over egui for the face

**Status**: Accepted

## Context

The face (layer 2) is a small (200×120) always-on-top window that renders an expressive ASCII character plus an optional hover panel with context. It needs:

- Native window, configurable position, optional always-on-top
- Transparent background
- Cross-platform: macOS, Linux (Wayland + X11). Windows in v2.
- A "hover panel" with richer UI (search box, expanded "today" view) — closer to a webapp than a sprite renderer
- Streamed events from the daemon (already over a Unix socket)

Two viable Rust GUI paths:

1. **Tauri 2.x** — Rust backend + webview frontend (Svelte/React/etc.).
2. **egui (eframe)** — pure-Rust immediate-mode GUI, no webview.

## Decision

**Tauri 2.x.**

### Rationale

- The hover panel is "small webapp" territory. egui can render it, but composing rich UI (a search box, a context panel with markdown, a clickable list of open loops) is faster in HTML/CSS than in egui.
- Tauri's window API is more mature for Wayland edge cases (`wlr-layer-shell` support via plugins, transparent backgrounds, focus-stealing rules).
- The face is the **marketing surface**. Iteration speed on visual polish matters more than raw render performance. Webview iteration loop is faster than rebuild-Rust.
- Layer 3 (brain) eventually adds richer surfaces: searchable memory results, "today" summary panel. These are squarely webapp-shaped.
- Tauri binary size is acceptable for a desktop tool (~10–15 MB stripped).

### Rejected alternatives

| Alternative                | Reason rejected                                                    |
|----------------------------|--------------------------------------------------------------------|
| egui (eframe)              | Slower iteration on visual polish; layer 3 surfaces want HTML/CSS  |
| iced                       | Less Wayland coverage than Tauri; smaller ecosystem                |
| GTK4 via gtk-rs            | macOS support is poor; iteration loop is slow                      |
| Native Cocoa/Win32         | Doubles platform surface area for no real win                      |
| Pure ASCII in terminal     | Defeats the "always-on-top peripheral" point                       |
| Web app + browser launcher | The user shouldn't have a Chrome tab as their dev environment status display |

## Consequences

- Two languages in the face: Rust (Tauri commands) + TS (webview). Acceptable.
- Build pipeline has a Node/npm dependency (for the webview). We document this clearly in the contributor README.
- The face crate (`homn-face`) wraps Tauri commands; the actual UI is in `src-tauri/` (Tauri's convention) using Svelte. Choice of frontend framework is non-load-bearing — Svelte preferred for bundle size; React acceptable.
- Wayland window management remains a hard problem (see [risks/known-unknowns.md](../../risks/known-unknowns.md)). Tauri eases it but doesn't solve it.
- The pasted overview's "weeks 5–6" timeline for the face was optimistic regardless of GUI choice. Tauri specifically helps the iteration speed but doesn't change platform reality. Adjusted to weeks 5–10 in [phases/phase-2-face.md](../../phases/phase-2-face.md).
