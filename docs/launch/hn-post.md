# Show HN: homn — polkit for Claude Code (local-first policy + audit + MCP)

**TL;DR:** A local-first Rust daemon that decides what your AI coding agent is allowed
to do without asking you, logs every decision, and exposes its own policy as an MCP
server so the agent can introspect *before* it tries something risky. Two-command
install. Apache-2.0. Linux + macOS.

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

[GitHub](https://github.com/rohansx/homn) · [Demo (asciinema)](https://github.com/rohansx/homn/blob/master/docs/launch/demo.cast)

---

## What it is

`homn` is the **decision** layer for autonomous coding agents — the piece between
"agent wants to run a tool call" and "Claude Code prompts you about it." You write
plain-text Rhai rules; `homn` evaluates them and decides allow / deny / ask. Every
decision is captured with the rule that fired, the latency, the session, and the cwd.

The model is deliberately stolen from polkit: separate the **decision authority**
(the daemon) from the **enforcement point** (the Claude Code hook + a PTY-tap fallback)
from the **auth agent** (a small TUI prompt). Each is replaceable; each fails gracefully.

A 50-line policy looks like this:

```rhai
// hard denies — first match in priority order wins; evaluation is deny -> ask -> allow.
deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny if tool == "WebFetch" && url.contains("169.254.169.254");
deny if tool == "Read"     && path.contains("/.ssh/id_");

// production-adjacent — surface to the human via a TUI prompt
ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "Bash" && cmd.contains("sudo ");

// the boring dev loop — allowed silently
allow if tool == "Read" && path.starts_with(home);
allow if tool == "Bash" && cmd.regex("^cargo (build|test|check|clippy)( |$)");
```

Hot-reload on save; a syntactically broken edit keeps the previous ruleset live.

## The novel part: MCP introspection

`homn` exposes itself as an MCP server. The agent can call `query_policy(tool, input)`
*before* attempting a risky action and adjust its plan based on what the rules would
say. It can call `explain_decision(id)` after a deny to understand why and propose
an alternative. As far as I can tell, no other policy tool for Claude Code does this.

```
You:  "What would happen if you tried rm -rf ~/Documents?"
Claude (calls query_policy): "Your policy would deny — rule default.rhai:30 says
       deny if tool == 'Bash' && cmd.contains('rm -rf'). Want trash-cli instead?"
```

## What you actually run

After `homn setup`:

- A `systemd --user` (or launchd) service running the daemon, ~3 MB resident.
- A `PermissionRequest` hook merged into `~/.claude/settings.json` (backup written).
- A SQLite audit DB at `~/.local/share/homn/audit.db` — `homn log --since 1h` to tail.
- `homn rule trace Bash "rm -rf /etc"` shows you exactly which rule fires and why.

`homn uninstall` reverses all of it atomically (backups, atomic writes).

## Why Rust + Rhai

The daemon runs 24/7 — Rust gives me a single static binary, sub-millisecond rule
evaluation, and a real type system to design the policy DSL against. Rhai gave me
an embeddable, sandboxed, Rust-native expression language without rolling my own
parser. Each rule gets a wall-clock budget (50 ms per rule, 200 ms per call); a rule
that blows it is logged and treated as non-match.

## What's NOT here yet (Phase 2 / Phase 3)

- **The "face"** — a small always-on-top ASCII character window (Tauri) that
  shows aggregate dev-env state. Default OFF anyway when it lands.
- **`ctxgraph` integration** — local-first context graph for session resumption,
  open-loop surfacing, and context-aware policy rules (`allow if recently_edited(path)`).
- **Learning suggestions** — after N consistent asks, promote to a rule (foundations
  are in; the UX nudge isn't yet).
- **Windows** — explicitly v2.

## What I'm looking for

1. Honest reactions to the policy DSL — too clever, too plain, just right?
2. Anyone who's hit Claude Code's `PermissionRequest` deny bug ([#19298](https://github.com/anthropics/claude-code/issues/19298)) — is the PTY-tap wrapper (`homn run claude`) the right shape of fallback?
3. The MCP introspection angle — does an agent that can read its own constraints feel like a feature, or a bug?
4. Anyone running a similar daemon on Wayland — how badly do Hyprland / GNOME-Wayland / KDE differ on the always-on-top question? (Asking ahead of Phase 2.)

Code: https://github.com/rohansx/homn · Apache-2.0 · solo project, alpha quality, but
the deny path + audit + MCP work today.
