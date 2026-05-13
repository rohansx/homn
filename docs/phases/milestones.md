# Milestones & ship dates

> Single-page view of every named milestone, its exit criteria, and the launch artifact it produces. Updated as phases ship.

## Calendar

Assumes work starts 2026-W20 (May 11). Adjusted from the pasted overview's optimistic 11-week target to 20 weeks for the full vision; the Phase 1 ship date is unchanged.

| Phase | Window      | Calendar (approx)         | Ship                              |
|-------|-------------|---------------------------|-----------------------------------|
| 1     | Weeks 1–4   | 2026-05-11 → 2026-06-08   | Policy engine v1 (HN launch)      |
| 2     | Weeks 5–10  | 2026-06-09 → 2026-07-20   | Face v0 (Product Hunt + demo video)|
| 3a    | Weeks 11    | 2026-07-21 → 2026-07-27   | ctxgraph readiness audit          |
| 3b    | Weeks 12–20 | 2026-07-28 → 2026-09-28   | Brain v0 (blog post + case study) |

## Per-phase exit criteria (recap)

### Phase 1 — Policy engine

| Criterion                                             | Status  |
|-------------------------------------------------------|---------|
| Daemon runs ≥7 days on author's machine, no crashes   | pending |
| ≥1000 decisions in author's audit.db                  | pending |
| Author reads `homn log --denied` weekly               | pending |
| ≥3 external users with custom rule files              | pending |
| ≥100 GitHub stars within 30 days of HN launch         | pending |

### Phase 2 — The face

| Criterion                                             | Status  |
|-------------------------------------------------------|---------|
| Installs cleanly on macOS + Hyprland + GNOME-Wayland  | pending |
| Demo video has ≥5,000 views                           | pending |
| Demo video comments don't use "cute" as primary adj.  | pending |
| ≥50% of opt-ins still using face 30 days later        | pending |
| ≥1,000 cumulative GitHub stars                        | pending |

### Phase 3 — The brain

| Criterion                                             | Status  |
|-------------------------------------------------------|---------|
| Week 11 ctxgraph readiness audit completed            | pending |
| Session-resume click-through >30%                     | pending |
| Open-loop nudge dismissal <40%                        | pending |
| ≥30% of opt-ins run ≥1 brain query per week           | pending |
| ctxgraph repo gains ≥250 stars within 30 days of launch | pending |

## Launch artifacts

| Phase | Artifact                                              | Audience                                  |
|-------|-------------------------------------------------------|-------------------------------------------|
| 1     | HN post: "polkit for Claude Code"                     | Devs running Claude Code, local-first folks |
| 1     | asciinema: `homn log` + `homn run claude` blocking destructive call | Same |
| 1     | Blog post: "two weeks of audit logs" (day +14)        | Same                                      |
| 2     | 90s demo video                                        | Broader dev Twitter, agentic-AI crowd     |
| 2     | Product Hunt + HN re-launch ("Show HN: I added a face") | Same                                    |
| 2     | Twitter thread with face GIFs                         | Same                                      |
| 3     | Blog post: "the brain inside homn"                    | RAG / agentic-AI infra builders           |
| 3     | ctxgraph case study                                   | Ctxgraph standalone audience              |
| 3     | Demo video v2                                         | Same                                      |

## Open issues blocking each phase

### Blocking Phase 1

- None known. Begin Week 1 with skeleton.

### Blocking Phase 2

- Tauri 2.x `wlr-layer-shell` plugin status as of Phase 2 start (verify before Week 5)

### Blocking Phase 3

- ctxgraph readiness audit (this is *the* gate; do it Week 11 before any code)
- Anthropic resolution status on transcript-content privacy guidance (do they document a recommended approach?)

## Dependencies / decision pressure on ourselves

If `claude agents` ships a "permission dashboard" feature that closely overlaps Phase 1, **we keep going** — `homn` is a peer process, not a UI replacement. The audit log + MCP introspection + Rhai rules are independent value even if Claude ships its own dashboard. Document this in the launch post explicitly.

If `claude agents` ships an official memory/recall feature, **we still keep going** for Phase 3 — `ctxgraph` is local-first, bi-temporal, and yours. The official feature will be cloud-tied and Anthropic-only; the local-first thesis still holds.

If the PermissionRequest hook bug (#19298) gets fixed during Phase 1, **we ship the PTY wrapper anyway** as belt-and-suspenders and document it as optional. Removing it later is easy; adding it under launch deadline pressure is hard.
