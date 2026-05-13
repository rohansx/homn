# Architecture — Layer 2: The Face

> The analog: Clippy, but useful, and not annoying. Default OFF in v1. Opt-in. Marketing surface, not retention surface.

## What it does

A small (200×120) always-on-top window that shows an expressive ASCII character. The character is a **live status display** for the aggregate state of your dev environment — sessions working, sessions waiting, builds passing, builds failing, PRs landing, idle, errored.

It is **not** a notification toast (those disappear). It is **not** a system tray icon (those are too small to encode state). It's the size of a webcam preview — small enough to live in a corner, large enough to glance at.

## Default OFF (deliberate)

The pasted brm overview defaulted the face ON. We're changing that: **the face is opt-in in v1.**

Why:
- Engineers configure away anything that demands attention.
- Audit log + TUI prompt is the daily-use surface for the policy crowd.
- The face is the marketing surface — what we record demo videos around — but it shouldn't be the only way to derive value from `homn`.
- This sidesteps the #1 product risk (face fatigue, doc-acknowledged in the original overview).

User opts in with `homn face --enable`. The face autostarts in subsequent sessions until they `homn face --disable`.

## Why expressive (rather than minimal / snarky)

Three rejected alternatives:

- **Calm / minimal.** Users forget the window exists; the state-encoding value is lost. Once you've decided to put a window on screen, it has to earn its space.
- **Snarky / funny.** Novelty wears off in a week. Then it becomes annoying. Especially for the engineering audience, who will mute or kill anything that feels like a personality nag.
- **Expressive (the pick).** The character encodes information without editorializing. Reactions, no opinions. Holds up over months of use.

The distinction from "badclaude" (whip-crack pressure tools): `homn`'s face is **signal**, not pressure. It tells you *what's happening*. It never tells the agent *to do anything*. The face has reactions; it has no commentary.

## State vocabulary

Placeholder art — final design will iterate. The point of this section is the **vocabulary**, not the glyphs.

```
   ◕ ◡ ◕        idle, mild head-bob
   /|_|\        all sessions calm, nothing waiting

   ◔_◔          tracking, mild concentration
   /|_|\        one or more sessions actively working

   ◉ ◉          eyes wide — a session needs permission
   /|⚐|\        (v0: shows session name in card; v2: spatial pointing)

   x_x          eyes shut, cross marks — a session errored
   /|_|\        last error visible on hover

   ◕‿◕          quick smile (~2 sec)
   /|_|\        task completed / CI passed / commit landed

   ⊙_⊙          alert, exclamation — high-stakes action waiting
   /|!|\        (git push to main, deploy, prod-adjacent network call)

   ¬_¬          mild eyebrow raise
   /|_|\        you've been on the same file 25+ min — stuck?

   z_z          dozing — no activity for an hour
   /|_|\        daemon still watching
```

## Reactions tied to real events (nothing random)

| Event source                                  | Reaction                                            |
|-----------------------------------------------|-----------------------------------------------------|
| Claude Code `PermissionRequest`               | `◉ ◉` — card with session name                      |
| `git commit` in watched repo                  | quick nod, brief commit hash overlay                |
| `cargo build` / `npm build` fails             | `x_x`, error count visible                          |
| PR opened by you                              | quick celebrate                                     |
| PR assigned to you by someone else            | gentle wave, hover shows PR title                   |
| 25 min on same file (file watcher)            | `¬_¬`, hover shows "you've been on X for 27m"       |
| Daemon notices an open loop (layer 3)         | thought bubble (e.g. "naman email still draft")     |
| 5pm / configured EOD                          | wave goodbye, summarizes the day on click           |

**The face is silent unless something changes.** No sound. No movement when state is stable. Expressive ≠ noisy.

## Interactions

| Gesture          | Action                                                            |
|------------------|-------------------------------------------------------------------|
| hover            | Context panel: which session needs you, what error, what PR, what loop |
| click on alert   | Opens the relevant decision card directly                         |
| right-click      | Menu: pause face, settings, mute 1h, open audit log               |
| drag             | Reposition window                                                 |
| double-click     | Expand to "today" panel: sessions, commits, open loops, day's audit |

## Why not just system notifications?

Doesn't scale to the workload. Specifically:

- Native notifications stack and disappear. The face is persistent state. Glance → know.
- Notifications can't aggregate. The face *is* the aggregate.
- Notifications can't degrade gracefully. The face mute mode is "summary mode" — same window, single neutral glyph, no animations.

## Why not the system tray?

Tray icons are too small (16×16 px). They can encode one state (icon swap) but not "two sessions working, one waiting, one errored, builds passing". The face is 200×120 — enough surface area to represent compound state.

## Implementation

- **Tauri** (Rust backend + webview frontend). Rationale in [ADR-0004](adr/0004-tauri-vs-egui.md).
- Character renders as **inline SVG** in the webview; animations driven by daemon events streamed over the event socket.
- Transparent background, optional always-on-top, configurable position.
- Platforms: macOS, Linux (Wayland via `wlr-layer-shell` where available, X11 fallback). Windows in v2.

### Window-management gotchas

Always-on-top semantics differ by compositor. The face uses, in order of preference:

1. **wlr-layer-shell** (`OVERLAY` layer): true overlay, draws above fullscreen apps. Hyprland, Sway, river.
2. **xdg-toplevel** with `setAlwaysOnTop(true)`: works on most GNOME-Wayland, mixed on KDE Plasma 6.
3. **X11 `_NET_WM_STATE_ABOVE`**: legacy fallback.
4. If nothing works, the face falls back to a regular window and the user can configure their compositor.

The face never tries to "force" on top — if the compositor says no, the audit log + ntfy fallback still surface the prompt.

## Spatial session-pointing — NOT in v0

The pasted overview proposed pointing the face at "the terminal that has the prompt". This requires cross-process window position introspection, which Wayland actively forbids for non-privileged clients.

**v0**: show session name in the card and on hover. Done.
**v2**: optional, behind a flag, X11-only, with appropriate caveats.

This is a deliberate scope cut. The "pointing" gesture is cute; the platform tax is not worth it for v0.

## Subscriber model — not a publisher

The face is a **read-only consumer** of the event bus. It cannot trigger policy decisions, write to ctxgraph, or change daemon state. It can:

- Subscribe to `BusEvent` over the events socket.
- Render the current aggregate state.
- Forward user clicks/keys back as decision answers (response to an in-flight `AskOpened` event).

The face crashing has no effect on the policy engine or the audit log. Running `homn` without the face is fully supported and the default in v1.

## Phase 2 exit criteria

Detailed in [phases/phase-2-face.md](../phases/phase-2-face.md). Top three:

1. The face installs cleanly on macOS + Hyprland + GNOME-Wayland in a single `cargo install` flow.
2. Launch demo video has ≥5,000 views and the comments don't include the word "cute" (signal that the face is read as useful, not decorative).
3. Among Phase 1 installs that opted into the face, ≥50% are still running it 30 days later.
