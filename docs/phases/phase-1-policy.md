# Phase 1 — Policy engine

> Weeks 1–4. Ship a useful tool that solves *"I want to stop babysitting permission prompts."*

## What ships

The daemon, the rules engine, the audit log, the TUI prompt, the MCP server, the install snippet, the PTY-tap wrapper. Face is opt-in (default OFF — see [architecture/face.md](../architecture/face.md)); Phase 1 does not require it.

### Concretely, in your installed `homn`

```
$ homn --version
homn 0.1.0

$ homn install                    # prints the JSON snippet for ~/.claude/settings.json
$ homn install --apply            # writes it

$ homn daemon                     # foreground; systemd user unit installs as default
$ systemctl --user status homn

$ claude                          # using your normal claude code — hook path
$ homn run claude                 # PTY-tap wrapper for guaranteed deny

$ homn rule list
$ homn rule edit                  # opens $EDITOR on policies/default.rhai
$ homn rule add 'allow if tool == "Read" && path.starts_with(home + "/dev")'
$ homn rule explain <decision_id>

$ homn log                        # tail audit
$ homn log --denied --since 24h
$ homn log --grep "supabase"

$ homn mcp stdio                  # exposes MCP server for claude code config
```

## Milestone breakdown

### Week 1 — Skeleton

- Cargo workspace, crate boundaries from [architecture/overview.md](../architecture/overview.md)
- Tokio main loop, Unix socket listener, JSON-line RPC dispatch
- `homn daemon` command boots, accepts connections, logs them
- Smoke test: connect with `socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/homn.sock` and exchange a `ping`

**Exit:** daemon stays up 1 hour without panicking under load.

### Week 2 — Policy + Audit

- Rhai engine integration, rule loader, hot-reload via inotify
- `default.rhai` with ~30 starter rules
- Per-rule and per-call wall-clock budgets enforced
- Audit schema, decisions write path, `decisions.list` query method
- `homn rule list`, `homn rule add`, `homn rule edit` CLI
- `homn log` CLI with `--since`, `--denied`, `--grep`, `--json`
- TUI prompt rendering with `ratatui` for the `ask` path
- Hook subcommand: `homn hook permission-request` + install snippet

**Exit:** can run `claude` against `homn` daemon; allow/deny/ask all work for at least the Bash and Read tools.

### Week 3 — PTY wrapper + Learning

- PTY wrapper: `homn run claude` spawns `claude` under a PTY, taps prompt, synthesizes y/n
- Learning subsystem: pattern frequency tracker → suggestion table
- `homn learning list` / `homn learning accept N` / `homn learning reject N` CLI
- Suggestions trigger a non-blocking TUI nudge after `ask` decisions
- MCP server v0: stdio transport, `query_policy`, `explain_decision`, `recent_decisions`, `suggest_rule`

**Exit:** the author runs `homn run claude` daily for 1 week and reports satisfaction.

### Week 4 — Polish, install, launch prep

- ntfy mirror (optional surface; configurable topic + idle threshold)
- systemd user unit + launchd plist for macOS (installer writes the right one)
- Graceful daemon restart (preserves in-flight `ask` state via socket reconnection by surfaces)
- Documentation pass: `README.md`, `docs/getting-started.md`, sample policy files
- Comparison post draft: "polkit for coding agents" — what existing tools don't do
- HN launch post draft + screenshots + ascii cinema of `homn log`

**Exit:** the author can `cargo install homn` on a fresh CachyOS install + macOS, configure in under 2 minutes, and have a working daemon.

## Out of scope for Phase 1

These belong to later phases. Explicit so we don't drift:

- The face (Tauri window). Phase 2.
- ctxgraph integration. Phase 3.
- Context-aware policy rules (`ctxgraph.has_open_pr_passing_ci(...)`). Phase 3.
- Team sync (paid plan). Post-Phase-3.
- Windows support. v2.
- Browser extension for tab tracking. Phase 3+.
- A web UI for the audit log. Maybe never — `homn log` is enough.

## Success metrics

| Metric                                                 | Target (30 days post-launch)               |
|--------------------------------------------------------|--------------------------------------------|
| GitHub stars                                            | ≥100                                       |
| Independent installs (verified via opt-in telemetry, OFF by default) | ≥30                          |
| External users writing custom rules                    | ≥3                                         |
| HN post position                                       | Top 10 on launch day                       |
| Daemon stability                                       | 0 unplanned crashes across users in 30d    |
| Author's own audit log reads / week                    | ≥1 (signal that audit is the killer feature)|

## Risks (Phase 1–specific)

- **PermissionRequest deny bug ([#19298](https://github.com/anthropics/claude-code/issues/19298))** — pre-mitigated via PTY wrapper. Track the upstream issue; remove the wrapper requirement when fixed.
- **Rhai performance under heavy use** — set Operations cap conservatively, add latency telemetry, optimize hot rules.
- **Hook reliability across Claude Code versions** — pin tested versions in install snippet, document the supported range.

Full risks list: [risks/known-unknowns.md](../risks/known-unknowns.md).

## Launch plan

- **Day -3**: README freeze, screenshots, asciinema of `homn log` and `homn run claude` blocking a `rm -rf`.
- **Day -1**: post the comparison piece to author's blog/dev.to.
- **Day 0** (Tuesday, 8am PT): HN submission "polkit for Claude Code". Twitter thread with the asciinema gif.
- **Day +1 to +7**: respond to every GitHub issue + reddit comment.
- **Day +14**: write a "two weeks of audit logs" follow-up post — what surprised the author about their own approval patterns.
