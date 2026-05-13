# Phase 2 — The expressive face

> Weeks 5–10. Ship the marketing surface that makes the daemon viral. Default OFF; opt-in.

## What ships

The Tauri window. 8 core animations. Hover panel. Event subscription. Mute mode. Tray-fallback. The full launch demo.

### Concretely

```
$ homn face --enable       # opt-in; sets autostart for this session
$ homn face                # launches the window in current session
$ homn face --mute 1h
$ homn face --mute 4h --until 18:00
$ homn face --disable
```

The face stays opt-in (per [ADR-0004](../architecture/adr/0004-tauri-vs-egui.md) + [architecture/face.md](../architecture/face.md)). Users who never enable it still benefit from everything Phase 1 shipped.

## Milestone breakdown

### Weeks 5–6 — Tauri skeleton + character

- Tauri 2.x project, transparent always-on-top window, drag-to-reposition
- Wayland `wlr-layer-shell` integration via Tauri plugin (Hyprland, Sway); xdg-toplevel fallback
- SVG character renderer; first 4 states implemented (idle / tracking / alert / dozing)
- Webview ↔ daemon event socket bridge
- Smoke test: face renders on macOS + Hyprland + GNOME-Wayland

**Exit:** the face boots, animates between two states on real `BusEvent`s.

### Weeks 7–8 — Full vocabulary + hover panel

- All 8 states from [architecture/face.md](../architecture/face.md)
- Hover panel: which session needs you, last error, today's audit summary
- Click-on-alert opens the in-flight decision card
- Right-click menu: mute, settings, audit log, quit
- "Mute for 1h" / "summary mode" (single-glyph minimal face) implemented
- Tray icon fallback for compositors where always-on-top fails

**Exit:** the face is usable 8 hours a day on the author's machine without annoying them.

### Weeks 9–10 — Polish + launch prep

- Position persistence across reboots
- Multi-monitor handling
- Configurable color theme (dark/light/custom)
- Settings UI accessible from right-click menu (toml editor backed by `homn.toml`)
- Demo video: 90 seconds, narrated by the author, recorded at 1440p
- Twitter thread + Product Hunt submission draft
- Press: reach out to Anthropic devrel, agentic-AI YouTubers

**Exit:** demo video is something the author is proud of.

## Out of scope for Phase 2

- Spatial session-pointing (face physically points at the terminal that needs you). Wayland forbids the introspection; see [risks/known-unknowns.md](../risks/known-unknowns.md). Maybe v2.
- Custom character art per user. Maybe v2 as a community asset thing.
- Voice / sound. Never (per principle, the face is silent).
- Windows support. v2.
- Layer 3 features (open loops, session resumption). Phase 3.

## Success metrics

| Metric                                              | Target (30 days post-launch)               |
|-----------------------------------------------------|--------------------------------------------|
| Demo video views                                    | ≥5,000                                     |
| Product Hunt position on launch day                 | Top 5                                      |
| GitHub stars (cumulative since Phase 1)             | ≥1,000                                     |
| Daily active use among opt-ins                      | ≥50% retention at day 30                   |
| Reviews / mentions that *don't* use the word "cute" | ≥80% of mentions read it as useful, not decorative |
| Reports of "face fatigue" (users disabling after 1 wk) | <20%                                    |

The "cute" metric matters. If the dominant adjective in reviews is "cute" rather than "useful", we built decoration, not utility.

## Risks (Phase 2-specific)

- **Wayland window management divergence** — Hyprland, Sway, GNOME-Wayland, KDE Plasma 6 all behave differently for always-on-top. Tauri 2.x helps; we still expect 1–2 weeks of platform-specific bug-fixing.
- **Face fatigue** — biggest product risk. Mitigations: default OFF, conservative animation cadence, "summary mode", 1h/4h mute hotkey.
- **Demo failure** — if the demo video doesn't go viral, Phase 2 doesn't unlock the audience that funds Phase 3 attention. Plan B: a written deep-dive post + asciinema (no video) launches the same week as a fallback.
- **Tauri bundle size** — ~12 MB stripped. Some users will complain; document the tradeoff (alternative was egui — slower iteration).

## What this phase is NOT trying to do

- It's **not** trying to be a daily-driver retention tool. The audit log + policy rules from Phase 1 do that work.
- It's **not** trying to be a replacement for `claude agents`. Different mental model: dashboard vs peripheral.
- It's **not** trying to be funny. State display, not personality.

## Launch plan

- **Day -7**: rough cut of demo video to ~5 trusted reviewers; iterate.
- **Day -1**: video finalized, Product Hunt submission scheduled, Twitter thread drafted.
- **Day 0** (Wednesday, 12:01 AM PT): Product Hunt post goes live. Twitter thread at 8 AM PT. HN re-launch with "Show HN: I added a face to my permission daemon, here's the demo".
- **Day +1 to +7**: respond to every comment; ship hotfixes daily.
- **Day +14**: write the "what surprised me about the face" follow-up.
