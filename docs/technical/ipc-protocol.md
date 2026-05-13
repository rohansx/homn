# Technical — IPC protocol

> The Unix socket wire format. Two sockets: request-response (`homn.sock`) and event broadcast (`homn-events.sock`).

## Sockets

```
$XDG_RUNTIME_DIR/homn.sock         # request-response, JSON-line RPC
$XDG_RUNTIME_DIR/homn-events.sock  # broadcast, SSE-style event stream
```

Both are SOCK_STREAM Unix sockets owned by the user, mode `0600`. No privileged ops; multiple connections allowed.

## Request-response (`homn.sock`)

JSON-line RPC. Each request is a single line of JSON followed by `\n`; each response is a single line of JSON followed by `\n`. Multiple requests per connection allowed.

### Request envelope

```json
{
  "id": "01HXY...",
  "method": "decisions.create",
  "params": { ... }
}
```

### Response envelope

```json
{
  "id": "01HXY...",
  "result": { ... }
}
```

or error:

```json
{
  "id": "01HXY...",
  "error": { "code": "policy_unavailable", "message": "..." }
}
```

## Methods

### `decisions.create`

Request the daemon to evaluate a policy decision. Used by both the hook path and the PTY wrapper.

```json
{
  "method": "decisions.create",
  "params": {
    "source": "hook" | "pty-wrapper" | "mcp",
    "session_id": "01HXY...",
    "cwd": "/home/rsx/dev/cloakpipe",
    "tool_name": "Bash",
    "tool_input": { "command": "git push origin main" },
    "permission_suggestions": [...],          // from claude code, optional
    "wait_for_human": true                    // if true, daemon blocks up to timeout
  }
}
```

Response (deterministic decision):

```json
{
  "result": {
    "decision_id": 42,
    "decision": "allow",
    "rule_source": "policies/default.rhai:14",
    "rule_text": "allow if tool == \"Bash\" && cmd.matches(\"git push origin *\")",
    "context": { "ctxgraph_hit": null }
  }
}
```

Response (deferred to human, with wait_for_human=true):

```json
{
  "result": {
    "decision_id": 43,
    "decision": "ask",
    "human_answer": "deny",
    "answered_by": "face",
    "latency_ms": 1820,
    "rule_source": null,
    "context": { "ctxgraph_hit": { "page": "wiki/concepts/release-process", "excerpt": "..." } }
  }
}
```

### `decisions.list`

Tail the audit log (used by `homn log` CLI).

```json
{
  "method": "decisions.list",
  "params": {
    "since": "2026-05-13T00:00:00Z",
    "decision": ["deny"],
    "session_id": null,
    "limit": 100
  }
}
```

### `policies.reload`

Force a hot reload of policy files.

### `learning.suggestions`

List pending learning suggestions for the user.

### `learning.accept` / `learning.reject`

Accept or reject a suggestion. Accept appends to the relevant policy file.

### `surfaces.register`

Used by face / TUI prompt to declare itself a subscriber. After registering, the surface SHOULD also connect to `homn-events.sock` to receive `BusEvent`s.

```json
{
  "method": "surfaces.register",
  "params": { "kind": "face", "version": "0.1.0", "capabilities": ["render-card", "render-toast"] }
}
```

### `surfaces.answer`

A surface forwards a human's decision answer:

```json
{
  "method": "surfaces.answer",
  "params": {
    "decision_id": 43,
    "answer": "allow" | "deny" | "always_allow" | "always_deny",
    "comment": "user clicked through after reading wiki excerpt"
  }
}
```

## Event broadcast (`homn-events.sock`)

After connecting, the client sends an optional filter line, then receives newline-delimited JSON events.

### Subscription request

```json
{ "filter": { "kinds": ["AskOpened", "AskClosed", "DecisionMade", "OpenLoopNudge"] } }
```

Send `{}` for "all events".

### Event format

```json
{
  "kind": "AskOpened",
  "ts": 1715587200,
  "payload": {
    "decision_id": 43,
    "tool_name": "Bash",
    "tool_input_preview": "git push origin main",
    "cwd": "/home/rsx/dev/cloakpipe",
    "session_id": "01HXY...",
    "context": { "ctxgraph_hit": { ... } }
  }
}
```

### Event kinds

| Kind                  | Emitted when                                                |
|-----------------------|-------------------------------------------------------------|
| `DecisionMade`        | Deterministic allow/deny resolved (no human asked)          |
| `AskOpened`           | A human-decision card is opened on a surface                |
| `AskClosed`           | A human answered (or timeout expired)                       |
| `LearningSuggestion`  | A new rule promotion is available                           |
| `SessionStarted`      | Claude session start hook fired                             |
| `SessionEnded`        | Claude session stop hook fired                              |
| `SessionResumeOffer`  | Phase 3: ctxgraph found resumable context for this session  |
| `OpenLoopNudge`       | Phase 3: an open loop is worth surfacing                    |
| `BuildPassed` / `BuildFailed` | Phase 3: build event from a wrapped command         |
| `CommitLanded`        | Phase 3: git commit detected in a watched repo              |
| `HighStakesPending`   | A `git push main`-class decision is pending                 |

## Concurrency model

- The daemon uses Tokio. Each connection is a task.
- The audit DB has a single writer task; readers go through a connection pool.
- The event bus is a `tokio::sync::broadcast` channel; subscribers see a snapshot of buffered events on subscribe.
- Policy evaluation runs on a small thread pool to enforce wall-clock budgets via timeouts.

## Why not D-Bus?

Considered. Rejected. See [research/polkit-deep-dive.md](../research/polkit-deep-dive.md). Summary: D-Bus is heavyweight when you don't need its main feature (cross-security-domain RPC). `homn` is a single-user daemon — Unix sockets are simpler, faster, easier to debug, and don't pull in `dbus-rs` dependencies.

## Why JSON-line?

- Trivial to debug (`socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/homn.sock`).
- Works with any language that has a JSON parser.
- No schema lock-in like Protobuf/Cap'n Proto.
- Cost is acceptable: hot path is policy eval (μs), serialization is rounding error.
