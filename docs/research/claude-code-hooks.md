# Research — Claude Code hooks

> The integration surface for `homn`. What works, what's broken, what's likely to change.

## Hook events we care about

Claude Code exposes a hook system in `~/.claude/settings.json`. Twelve lifecycle events. The ones that matter for `homn`:

| Event              | Fires when                                         | We use it for           |
|--------------------|----------------------------------------------------|-------------------------|
| `PreToolUse`       | Before any tool call                               | First chance to gate    |
| `PermissionRequest`| User would normally see the permission prompt      | Primary policy hook     |
| `Notification`     | Claude waiting for input / idle ≥60s                | Wake the face           |
| `SessionStart`     | New session begins                                 | Ctxgraph "what was I doing here?" |
| `UserPromptSubmit` | User submits a prompt                              | Inject session-resume context |
| `PostToolUse`      | After a tool runs                                  | Audit success/fail      |
| `Stop`             | Session ends                                       | Wrap audit / save state |

## How a hook is wired

```json
// ~/.claude/settings.json
{
  "hooks": {
    "PermissionRequest": [{
      "matcher": "*",
      "hooks": [{
        "type": "command",
        "command": "homn hook permission-request",
        "timeout": 30000
      }]
    }]
  }
}
```

The hook is invoked with the tool payload on stdin as JSON. It returns a decision on stdout as JSON. Exit code 0 means "decision provided"; non-zero means "fall through to Claude's default behavior".

### PermissionRequest payload (what we receive)

```json
{
  "session_id": "01HXYZ...",
  "tool_name": "Bash",
  "tool_input": { "command": "git push origin main", "cwd": "/home/rsx/dev/cloakpipe" },
  "permission_suggestions": [
    { "type": "always_allow", "scope": "tool_name" },
    { "type": "always_allow", "scope": "project" }
  ]
}
```

Notice `permission_suggestions` — these are the "always allow this" options Claude would normally show. We can plumb them into our card's *Remember* button.

### PermissionRequest return (what we send back)

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": { "behavior": "allow" }
  }
}
```

`behavior` is `allow`, `deny`, or `ask`. (See bug below for `deny`.)

## The bug we have to design around: #19298

[anthropics/claude-code#19298](https://github.com/anthropics/claude-code/issues/19298) — **PermissionRequest hook decision is ignored when deny.**

What happens:
- Hook returns `{ behavior: "deny" }`.
- The decision is recorded in the audit log Claude keeps.
- The interactive permission prompt still appears in the terminal.
- The user can override the hook's deny by pressing `y`.

This means the polkit-style story — *"first matching rule wins, deny stops the call"* — breaks. Allow works. Deny is best-effort. We need a fallback.

**Our fallback: PTY-tap wrapper.** `homn run claude ...` spawns `claude` as a child process with a PTY, taps the output stream for the prompt pattern (`Do you want to proceed? (y/n):`), and races our decision against Claude's 5s default. If the daemon decides `deny` before the user can press `y`, the wrapper writes `n\n` to the child's stdin. Belt-and-suspenders.

The wrapper is opt-in (`homn run claude` vs `claude`), so users who trust the hook can skip the PTY tax. See [ADR-0003](../architecture/adr/0003-pty-fallback.md).

## Timeout semantics

- Default hook timeout: 2000ms.
- Configurable up to ~25s for LLM-based hooks.
- **We commit to 30s in our default config.** Enough for the human to read the card + decide, including a slow `ctxgraph` query for context (target p95 200ms but headroom for cold cache).

If we exceed the timeout, Claude falls through to its default prompt. This is a graceful degradation, not a failure.

## MCP server tools and permissions

MCP tools come through as `mcp__<server>__<tool>` in the tool_name. They participate in `PermissionRequest` like native tools. This means a single Rhai rule can gate "any MCP tool from server X":

```rhai
ask if tool.starts_with("mcp__supabase__") && !cwd.starts_with(home + "/dev/utkrushta");
```

## Background agents and the visibility gap

[anthropics/claude-code#25520](https://github.com/anthropics/claude-code/issues/25520) — when using `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`, background agent permission prompts fire the `PermissionRequest` hook correctly **but are invisible in the VS Code extension UI**.

This is actually a feature for `homn`: where the official UI fails to surface the prompt, `homn`'s face / TUI surface it. Genuine bug → real user value.

## Reading the hook config in homn

`homn` does NOT modify `~/.claude/settings.json` directly. Install flow:

1. `homn install` writes a recommended JSON snippet to stdout.
2. User pastes it into their settings.json (or runs `homn install --apply` for the brave).
3. The snippet calls `homn hook <event-name>` for each event we care about.
4. The hook is a thin CLI subcommand of `homn` (same binary, different entry point) that POSTs to the daemon's Unix socket and writes the daemon's response to stdout in the right hook return format.

This keeps the daemon decoupled from Claude's settings format — if Anthropic changes the schema, we update the install snippet, the daemon stays untouched.

## Sources

- [Claude Code Docs — Hooks reference](https://code.claude.com/docs/en/hooks)
- [Claude Code Docs — Permissions](https://code.claude.com/docs/en/permissions)
- [Issue #19298 — PermissionRequest deny ignored](https://github.com/anthropics/claude-code/issues/19298)
- [Issue #25520 — Background-agent prompts invisible](https://github.com/anthropics/claude-code/issues/25520)
- [doobidoo's Universal Permission Request Hook gist](https://gist.github.com/doobidoo/fa84d31c0819a9faace345ca227b268f) — prior art, what people do today
- [alexop.dev — Claude Code notification hooks setup](https://alexop.dev/posts/claude-code-notification-hooks/)
