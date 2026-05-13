# Technical — Claude Code hook integration

> The wire-level details of how `homn` plugs into Claude Code. Read this alongside [research/claude-code-hooks.md](../research/claude-code-hooks.md).

## Install snippet

`homn install` prints a recommended snippet for `~/.claude/settings.json` and (optionally, with `--apply`) writes it for you:

```json
{
  "hooks": {
    "PermissionRequest": [
      {
        "matcher": "*",
        "hooks": [{
          "type": "command",
          "command": "homn hook permission-request",
          "timeout": 30000
        }]
      }
    ],
    "Notification": [
      {
        "matcher": "*",
        "hooks": [{
          "type": "command",
          "command": "homn hook notification",
          "timeout": 2000
        }]
      }
    ],
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [{
          "type": "command",
          "command": "homn hook session-start",
          "timeout": 5000
        }]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": "*",
        "hooks": [{
          "type": "command",
          "command": "homn hook user-prompt-submit",
          "timeout": 3000
        }]
      }
    ],
    "Stop": [
      {
        "matcher": "*",
        "hooks": [{
          "type": "command",
          "command": "homn hook stop",
          "timeout": 2000
        }]
      }
    ]
  }
}
```

`homn install` is idempotent — it merges with existing hooks and skips entries already pointing at `homn hook *`.

## Hook subcommand contract

Each `homn hook <event>` subcommand:

1. Reads the JSON payload from stdin.
2. Connects to the daemon socket at `$XDG_RUNTIME_DIR/homn.sock` (with one retry + 250ms backoff).
3. POSTs the event to the matching daemon route.
4. Receives a JSON response.
5. Translates to the hook-return format Claude expects on stdout.
6. Exits 0.

If the daemon is unreachable, the hook exits 0 with an empty response (Claude falls through to default behavior). If the daemon returns an error, the hook exits 0 with `{"decision": {"behavior": "ask"}}` — safer to ask than to silently allow.

## Per-event behavior

### `permission-request`

1. POST payload to `/decisions`.
2. Daemon evaluates rules.
3. If decision is `allow` or `deny`, return immediately.
4. If decision is `ask`:
   - Daemon broadcasts `AskOpened` on the event bus.
   - Subscribed surfaces (face, TUI, ntfy) compete to render.
   - Daemon waits up to ~28s (under the 30s hook timeout) for a human answer.
   - If answered: write to audit, feed learning, return the answer.
   - If timed out: return `{behavior: "ask"}` — Claude shows its own prompt as the final fallback.

### `notification`

1. POST payload to `/notifications`.
2. Daemon emits a face state change (e.g., `◉ ◉` if the notification is a permission idle ping).
3. Always exits 0 with empty response — `Notification` hooks don't influence Claude's behavior, only surface them.

### `session-start`

1. POST `{ session_id, cwd, model, timestamp }` to `/sessions`.
2. Daemon records the session, runs ctxgraph lookups, may emit a `SessionResumeOffer` BusEvent (Phase 3).
3. Hook returns empty.

### `user-prompt-submit`

1. POST payload to `/sessions/:id/prompts`.
2. If a `SessionResumeOffer` was accepted, daemon injects context summary into the response via Claude's hook injection contract:
   ```json
   {
     "hookSpecificOutput": {
       "hookEventName": "UserPromptSubmit",
       "additionalContext": "Context from last session in this repo: ..."
     }
   }
   ```
3. Otherwise return empty.

### `stop`

1. POST `{ session_id, ended_at, message_count }` to `/sessions/:id/stop`.
2. Daemon flushes any pending audit writes for this session, records end time, runs ctxgraph "wrap-up" job.
3. Always exits 0 empty.

## PTY-tap wrapper (`homn run claude ...`)

When the user invokes `homn run claude` instead of `claude`:

1. The wrapper allocates a PTY using `portable-pty`.
2. Spawns `claude` as a child process, inheriting argv from after `claude`.
3. The wrapper's stdout/stderr are passed through to the user's terminal verbatim (read-only).
4. A background task scans the stream for the prompt regex (configurable, default matches `Do you want to proceed? \(y/n\):`).
5. On match, the wrapper POSTs to `/decisions` in parallel with the hook (which also fires).
6. The daemon de-duplicates: if both paths report the same decision, only one entry lands in the audit log.
7. If the daemon returns `deny` faster than the user can press a key, the wrapper writes `n\n` to Claude's stdin.

The wrapper exits with the child process's exit code.

## Configuration knobs (in `~/.config/homn/homn.toml`)

```toml
[hook]
timeout_ms = 28000            # under Claude's 30s
fallback_decision = "ask"     # what to return if daemon errors

[pty_wrapper]
enabled = true                # invoking `homn run claude` works at all
prompt_regex = '''Do you want to proceed\? \(y/n\):'''
deny_race_window_ms = 200     # if daemon decides deny within this, synthesize 'n'

[surfaces]
default = "tui"               # "tui" | "face" | "auto"
face_enabled = false          # v1 default OFF (see ADR-0004 + face.md)
ntfy_topic = ""               # leave empty to disable
ntfy_after_idle_minutes = 5
```

## Versioning

The hook payload schema is owned by Claude Code. We pin a supported range in `homn`'s install command:

```
homn install --requires-claude-code ">=2.0,<3.0"
```

If the user is outside the range, `homn install` warns but still writes the snippet. Hook contract changes mid-range will be tracked in [research/claude-code-hooks.md](../research/claude-code-hooks.md) and addressed in patch releases.
