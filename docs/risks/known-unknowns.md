# Risks & known unknowns

> Honest list of what could break this. Refresh at each phase boundary.

## Tier 0 — Things that could kill the project

### PermissionRequest hook deny is broken upstream

[anthropics/claude-code#19298](https://github.com/anthropics/claude-code/issues/19298)

- **Risk**: hook returns `deny`, Claude shows the prompt anyway. Polkit-style guarantee breaks.
- **Mitigation**: PTY-tap wrapper as opt-in fallback (see [ADR-0003](../architecture/adr/0003-pty-fallback.md)). Phase 1 ships both paths.
- **Watch**: track the upstream issue. If Anthropic fixes mid-Phase-1, document the wrapper as belt-and-suspenders and continue shipping it.

### ctxgraph isn't actually ready for Phase 3

- **Risk**: assumed API surface, missing schema versioning, ingestor pattern needs work.
- **Mitigation**: Week-11 readiness audit is a hard gate before Phase 3 implementation. Filed issues against ctxgraph land first; sequence ahead of phase start.
- **Watch**: between Phase 2 ship and Week 11, run a "dry-run ingest" against a fake event source — if that's painful, the audit will show why.

### Anthropic deprecates the hook surface mid-product

- **Risk**: Claude Code v3 ships with a different hook contract; `homn`'s integration breaks.
- **Mitigation**: pinned-version range in `homn install`. Wire-level compatibility tests in CI against multiple Claude Code versions. Hook subcommands are versioned (`homn hook permission-request --schema 2`).
- **Watch**: subscribe to anthropics/claude-code releases.

## Tier 1 — Things that could meaningfully slow shipping

### Wayland always-on-top divergence

- **Risk**: Hyprland, Sway, GNOME-Wayland, KDE Plasma 6 all behave differently. wlr-layer-shell isn't universal.
- **Mitigation**: Tauri 2.x abstracts most of it; fallback chain documented in [architecture/face.md](../architecture/face.md). 2–3 weeks of platform-specific work budgeted in Phase 2.
- **Watch**: Tauri's window plugin issues, especially layer-shell support.

### Face fatigue (users disable it within a week)

- **Risk**: even with conservative cadence, the face becomes annoying.
- **Mitigation**: default OFF in v1 (deviation from the pasted overview). Opt-in. Mute hotkey. Summary-mode minimal glyph. Cadence dial in `homn.toml`.
- **Watch**: 7-day, 14-day, 30-day retention among opt-ins. <50% at 30 days = problem.

### Rhai eval blowing budgets

- **Risk**: user writes a rule that hangs the engine (regex catastrophic backtracking, recursive function calls).
- **Mitigation**: `set_max_operations(100_000)` + per-call wall-clock timeout. Rules that exceed → logged + treated as non-match. Engine-level enforcement, not OS-level.
- **Watch**: `homn log --since 7d --grep "budget_exceeded"`. If this fires more than rarely, default budgets are too tight or rules are pathological.

### SQLite WAL contention under high decision rate

- **Risk**: if author runs ~10 parallel sessions doing many tool calls each, audit writes contend.
- **Mitigation**: single-writer task with channel-based batching. WAL mode. Indexes only on lookup columns. Write batching at 10ms windows.
- **Watch**: `homn daemon --metrics` exposes write latency percentiles.

## Tier 2 — Things that could damage adoption

### Transcript ingestion privacy regression

- **Risk**: a redaction regex misses a new credential format; user's audit DB ends up containing secrets.
- **Mitigation**: opt-in default. Encryption at rest. `homn brain audit-redaction` CLI lets users preview what would be persisted. Bug bounty for redaction misses.
- **Watch**: each redaction pattern has a unit test; track Linear-style "redaction-miss" reports.

### Demo video doesn't go viral (Phase 2)

- **Risk**: hours of polish, <500 views.
- **Mitigation**: have a written-deep-dive Plan B ready the same week. Reach out to known agentic-AI reviewers a week before launch for "embargo" previews.
- **Watch**: 48h view count vs target.

### Engineers configure away anything that looks like a personality nag

- **Risk**: the face is described as "annoying" / "Clippy" in HN comments, kills viral lift.
- **Mitigation**: lead messaging with "default OFF, audit log + policy is the real product". Frame face as marketing surface, not retention surface. Use phrases like "ambient signal" not "personality".
- **Watch**: review comments on launch day. If "Clippy" appears more than "polkit-for-CC", positioning failed.

### Naming confusion / poor discoverability

- **Risk**: `homn` doesn't mean anything obvious; SEO for "homn" surfaces unrelated content.
- **Mitigation**: tagline ("the homunculus for your coding agents") explains in 6 words. README front-loads "polkit for Claude Code" for SEO.
- **Watch**: organic search traffic to `homn.dev` / GitHub repo. If "polkit claude code" → us > "homn" → us, the tagline is doing the heavy lifting.

## Tier 3 — Things to keep an eye on

### `claude agents` ships overlapping features

- **Risk**: Anthropic's official dashboard subsumes some of `homn`'s value.
- **Mitigation**: position `homn` as a *peer*, not a UI replacement. Audit log + MCP introspection + Rhai rules are independent value. We complement; we don't compete on UI.
- **Watch**: anthropic/claude-code release notes.

### MCP server abuse (an agent reads its own constraints and routes around them)

- **Risk**: agent calls `query_policy`, sees deny, modifies its approach to slip past the rule.
- **Mitigation**: this is a *feature*, not a bug. The audit log records the deny; the user reviews. If a user worries about this they can disable the MCP server (`homn mcp disable`).
- **Watch**: feedback from security-research community on threat model.

### Open-source team-sync (Phase 3+) competes with paid plan

- **Risk**: someone forks `homn` and ships a free team-sync feature; our open-core plan loses revenue motivation.
- **Mitigation**: paid plan adds hosting, signing, support — not just sync mechanics. License the paid component differently if needed.
- **Watch**: 6-month revenue from team plan; community forks of sync component.

### Windows users are vocal in HN comments

- **Risk**: every Show HN gets "needs Windows" complaints. Distracts from launch.
- **Mitigation**: explicit "Linux/macOS in v1, Windows in v2" in the README's first paragraph and the HN post's first line.
- **Watch**: comment ratio on launch day.

## Reviewing this list

Re-read at:
- End of Phase 1 (Week 4)
- End of Phase 2 (Week 10)
- Phase 3 readiness audit (Week 11)
- Each ship date

Remove items that have resolved; add new ones as they surface. This doc should always reflect what's *currently* keeping you up at night.
