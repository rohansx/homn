# Technical — MCP server

> The MCP surface exposed by `homn`. The most novel piece of the design — see [ADR-0006](../architecture/adr/0006-mcp-server.md).

## Transports

`homn` exposes its MCP server on:

- **stdio** (`homn mcp stdio`) — invoked by Claude Code's MCP config.
- **Streamable HTTP** (`homn mcp http --port NNNN`) — for cross-machine setups and `claude agents`-style multi-host scenarios.

Default port: 9874. Configurable in `~/.config/homn/homn.toml`.

## Adding to Claude Code

```json
// ~/.claude.json
{
  "mcpServers": {
    "homn": {
      "command": "homn",
      "args": ["mcp", "stdio"]
    }
  }
}
```

Or via `claude mcp add` — `homn install` prints the right command.

## Tools

### `query_policy`

> *"What would `homn` decide if I tried this call?"*

Dry-run evaluation. **Does not log to audit, does not feed learning.**

```json
{
  "name": "query_policy",
  "description": "Returns what decision the homn daemon would make for a given tool call without actually invoking it. Useful for the agent to reason about its constraints before attempting an action.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "tool": { "type": "string", "description": "e.g. 'Bash', 'WebFetch', 'mcp__supabase__query'" },
      "tool_input": { "type": "object" }
    },
    "required": ["tool", "tool_input"]
  }
}
```

Returns:

```json
{
  "decision": "deny",
  "rule_source": "policies/default.rhai:10",
  "rule_text": "deny if tool == \"Bash\" && cmd.contains(\"rm -rf\") && !cwd.starts_with(\"/tmp\")",
  "alternatives_to_try": [
    "Move target into /tmp first, then rm -rf there",
    "Use trash-cli (trash <path>) — typically allowed by reasonable policies"
  ]
}
```

The `alternatives_to_try` array is best-effort: pattern-based suggestions (no LLM call). Often empty.

### `explain_decision`

> *"Why was decision N decided that way?"*

```json
{
  "name": "explain_decision",
  "description": "Returns the rule that fired (if any), its source location, ctxgraph context that contributed, and the audit log entry for a given decision id.",
  "inputSchema": {
    "type": "object",
    "properties": { "decision_id": { "type": "integer" } },
    "required": ["decision_id"]
  }
}
```

### `suggest_rule`

> *"Give me a Rhai rule that would let this whole class of calls through."*

Looks at the pattern of recent denials/asks and proposes a Rhai rule. **Does not modify policy files** — the user must explicitly accept via `homn rule add`.

```json
{
  "name": "suggest_rule",
  "description": "Proposes a Rhai rule that would auto-allow (or auto-deny) a class of calls similar to the given example. Suggestion only — never modifies policy files.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "example_tool": { "type": "string" },
      "example_input": { "type": "object" },
      "verb": { "type": "string", "enum": ["allow", "deny", "ask"] }
    },
    "required": ["example_tool", "example_input", "verb"]
  }
}
```

### `recent_decisions`

> *"Tail the audit log."*

Filterable list of recent decisions. Useful for the agent to ask *"what did the user say no to in the last hour?"* and avoid repeating.

```json
{
  "name": "recent_decisions",
  "description": "Returns the most recent N decisions from the audit log, with optional filters by tool, decision, or session_id.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "limit": { "type": "integer", "default": 50, "maximum": 500 },
      "tool": { "type": "string" },
      "decision": { "type": "string", "enum": ["allow", "deny", "ask"] },
      "session_id": { "type": "string" },
      "since_seconds_ago": { "type": "integer" }
    }
  }
}
```

### `ctxgraph_*` (Phase 3, proxied)

`homn` re-exports ctxgraph's MCP tools through its own server so agents have a single MCP surface. We do not duplicate; we proxy.

| Proxied tool                   | Description                                |
|--------------------------------|--------------------------------------------|
| `ctxgraph_search(query)`       | FTS5 + entity-resolved search              |
| `ctxgraph_session_history(cwd)`| Recent sessions in this directory          |
| `ctxgraph_open_loops()`        | Things you started but didn't finish       |

## Resources

The MCP server also exposes a small set of read-only resources:

| URI                                | Returns                                       |
|------------------------------------|-----------------------------------------------|
| `homn://policies/default`          | The current `default.rhai` content            |
| `homn://policies/<project>`        | Project-scoped rules                          |
| `homn://audit/recent`              | Recent decisions, JSONL                       |
| `homn://learning/suggestions`      | Pending learning suggestions                  |

## Prompts

Optional prompt templates the agent can pull:

| Prompt name                | Purpose                                          |
|----------------------------|--------------------------------------------------|
| `before_destructive_call`  | "Before attempting `rm -rf`, query `query_policy` and consider alternatives." |
| `on_denied_call`           | "If a call was denied, use `explain_decision` to read the rule and propose an alternative approach." |

Loaded from `$XDG_CONFIG_HOME/homn/prompts/*.md`. Ships with sensible defaults.

## Security considerations

- The MCP server runs as the user; it can answer anything the user can see locally.
- `query_policy` and `explain_decision` are read-only — they don't reveal anything beyond what the audit log already records.
- `suggest_rule` doesn't modify policy. Even if an agent went rogue, it can't escalate.
- HTTP transport binds to localhost by default. Cross-machine usage requires explicit `--bind 0.0.0.0` and is documented as "you understand what you're doing."
