# Contract — Claude Code hook protocol

> Wire format for each `homn hook <event>` subcommand. Stdin is the hook payload; stdout is the hook return. Exit 0 always.

## PermissionRequest

### Stdin (payload from Claude Code)

```json
{
  "session_id": "01HXYZ...",
  "tool_name": "Bash",
  "tool_input": {
    "command": "git push origin main",
    "cwd": "/home/rsx/dev/cloakpipe"
  },
  "permission_suggestions": [
    { "type": "always_allow", "scope": "tool_name" },
    { "type": "always_allow", "scope": "project" }
  ]
}
```

### Stdout (return to Claude Code)

Allow:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": { "behavior": "allow" }
  }
}
```

Deny (note: per #19298, Claude Code currently ignores deny — but we still emit it for forward compatibility):

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": { "behavior": "deny" }
  }
}
```

Ask (defer to Claude's interactive prompt):

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": { "behavior": "ask" }
  }
}
```

### Daemon socket message (request)

```json
{
  "id": "01HXY...",
  "method": "decisions.create",
  "params": {
    "source": "hook",
    "session_id": "01HXYZ...",
    "cwd": "/home/rsx/dev/cloakpipe",
    "tool_name": "Bash",
    "tool_input": { "command": "git push origin main" },
    "permission_suggestions": [...],
    "wait_for_human": true
  }
}
```

### Daemon socket message (response — deterministic)

```json
{
  "id": "01HXY...",
  "result": {
    "decision_id": 42,
    "decision": "allow",
    "rule_source": { "file": "policies/default.rhai", "line": 14 },
    "rule_text": "allow if tool == \"Bash\" && cmd.matches(\"git push origin *\")",
    "context": null
  }
}
```

### Daemon socket message (response — `ask` resolved by human)

```json
{
  "id": "01HXY...",
  "result": {
    "decision_id": 43,
    "decision": "ask",
    "human_answer": "deny",
    "answered_by": "tui",
    "latency_ms": 1820,
    "rule_source": null,
    "context": null
  }
}
```

### Failure modes

| Failure                                  | Hook behavior                                   |
|------------------------------------------|-------------------------------------------------|
| Daemon socket missing                    | Retry once, 250 ms backoff. Then exit 0 empty.  |
| Daemon returns error                     | Exit 0 with `{decision: {behavior: "ask"}}`     |
| Daemon takes > `hook.timeout_ms`         | Exit 0 with `{decision: {behavior: "ask"}}`     |
| Stdin is not valid JSON                  | Exit 0 with `{decision: {behavior: "ask"}}`     |

Empty stdout is also valid — Claude falls through to default behavior.

## Notification

### Stdin

```json
{
  "session_id": "01HXY...",
  "kind": "permission-idle" | "input-idle",
  "tool_name": "Bash",
  "ts_seconds_since_idle": 67
}
```

### Stdout

Empty `{}`. `Notification` hooks don't influence Claude's behavior; they're informational. The daemon uses them to wake the face (Phase 2) or to mirror to ntfy after the idle threshold.

## SessionStart

### Stdin

```json
{
  "session_id": "01HXY...",
  "cwd": "/home/rsx/dev/cloakpipe",
  "model": "claude-opus-4-7",
  "started_at": "2026-05-13T14:23:01Z"
}
```

### Stdout

Empty `{}`. Daemon records the session and may emit a `SessionResumeOffer` BusEvent (Phase 3).

## UserPromptSubmit

### Stdin

```json
{
  "session_id": "01HXY...",
  "prompt": "..."
}
```

### Stdout — usually empty

If a `SessionResumeOffer` is pending and the user accepted it (recorded daemon-side), the hook injects context:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "Context from last session in this repo: ..."
  }
}
```

## Stop

### Stdin

```json
{
  "session_id": "01HXY...",
  "ended_at": "2026-05-13T15:42:11Z",
  "message_count": 47
}
```

### Stdout

Empty `{}`. Daemon flushes pending audit writes for this session and records end time.

## Compatibility

`homn install` writes hooks for a pinned range of Claude Code versions. The range is recorded in the install snippet's comment header:

```jsonc
{
  "hooks": {
    // homn install (compatible with claude-code >=2.0,<3.0; pinned 2026-05-13)
    "PermissionRequest": [ ... ]
  }
}
```

If `homn` detects a Claude Code outside the supported range at hook-invocation time, it logs a warning and degrades to `{behavior: "ask"}` for safety.
