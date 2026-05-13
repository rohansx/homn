# Architecture — Layer 1: Policy Engine

> The analog: `polkitd` for coding agents. The only layer that's load-bearing for "v1 is useful".

## What it does

1. Receive a tool-use intent from Claude Code (via hook or PTY tap).
2. Look up the right policy file (default + project-scoped overrides).
3. Evaluate Rhai rules in **deny → ask → allow** order; first match wins.
4. Persist the decision + rule that fired to the audit log.
5. Return the decision to the caller.
6. If the decision was *ask* and the human answered, log their answer + feed it into the learning subsystem.

## The rules file

Plain text, version-controllable, sandboxed. Lives at `$XDG_CONFIG_HOME/homn/policies/`.

```rhai
// default.rhai

// always allow reads in your own dirs
allow if tool == "Read" && path.starts_with(home);

// auto-allow common build/test commands
allow if tool == "Bash" && cmd.matches("npm run *");
allow if tool == "Bash" && cmd.matches("cargo (build|test|check) *");
allow if tool == "Bash" && cmd.matches("pytest *");

// always deny destructive commands outside scratch dirs
deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny if tool == "Bash" && cmd.matches("git push --force *");

// ask for production-adjacent stuff
ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "WebFetch" && url.contains("internal.");

// default: ask but learn
ask if true;
```

Project overrides at `policies/<repo-slug>.rhai`. Daemon picks the file based on the *cwd* of the calling session.

Full DSL reference: [technical/policy-language.md](../technical/policy-language.md).

## Evaluation order — and why it matches polkit

**deny → ask → allow.** First matching rule wins. Same as polkit's authority evaluation. Reasons:

1. Users don't have to learn a new mental model.
2. Deny is sticky: a destructive rule at the top of `default.rhai` can't be accidentally overridden by a permissive rule lower down.
3. It's the order that makes "audit log" make sense: *"this was denied because rule X fired before any allow could match."*

## Three new capabilities the daemon adds

### 1. Learning (suggestion-only)

Every `ask` decision the human resolves gets logged with full context. After N consistent answers for the same pattern (default 5, configurable per scope), `homn` surfaces a **promotion prompt**:

> *"You've allowed `git push origin feat/*` 7 times this week. Promote to rule?"*
>
> `[promote to allow]  [keep asking]  [show generated rule]`

Clicking *promote* appends a generated rule to the relevant policy file:

```rhai
// added by homn learning on 2026-05-13 — 7 consistent allows for this pattern
allow if tool == "Bash" && cmd.matches("git push origin feat/*");
```

The user owns the rule file. `homn` **never silently modifies policy** — every change is an offered suggestion that lands as a writable line of Rhai with a comment.

### 2. Audit log

Every decision goes to SQLite at `$XDG_DATA_HOME/homn/audit.db`. Schema:

```sql
CREATE TABLE decisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,                -- unix epoch millis
  session_id    TEXT NOT NULL,
  cwd           TEXT NOT NULL,
  tool_name     TEXT NOT NULL,
  tool_input    TEXT NOT NULL,                   -- json blob, capped 4KB
  decision      TEXT NOT NULL CHECK (decision IN ('allow', 'deny', 'ask')),
  human_answer  TEXT,                            -- nullable, set if decision='ask'
  rule_source   TEXT,                            -- nullable, file + line if a rule fired
  rule_text     TEXT,                            -- snapshot of the rule for retroactive readability
  latency_ms    INTEGER NOT NULL,
  surface       TEXT                             -- which surface answered: tui|face|ntfy|mcp
);

CREATE INDEX idx_decisions_ts ON decisions(ts);
CREATE INDEX idx_decisions_session ON decisions(session_id);
```

Queryable via the CLI:

```
$ homn log --since 1h --denied
[14:23:01] deny  Bash("rm -rf ~/projects/old")
           rule: policies/default.rhai:9 — "deny if cmd.contains(rm -rf) && !cwd.starts_with(/tmp)"
           session: cloakpipe-refactor

[14:31:45] deny  WebFetch("https://internal.utkrushta.io/api")
           rule: policies/default.rhai:14 — "deny if url.contains(internal.)"
           session: utkrushta-review
```

This is the killer feature. Opaque approve/deny is the failure mode of every existing tool — the audit log fixes it.

Retention: 30 days by default, configurable. Compaction job runs daily.

### 3. MCP server

`homn` exposes itself as an MCP server (via the `rmcp` crate). Tools surfaced:

- `query_policy(tool, tool_input)` → *what would happen if I tried this?* Returns the decision the daemon would make without actually logging it.
- `explain_decision(decision_id)` → *why was this decided?* Returns the rule text, location, and any ctxgraph hit that contributed.
- `suggest_rule(pattern)` → *what rule would let me do this whole class?* Returns a draft Rhai line the user could add to their policy.
- `recent_decisions(filters)` → tail the audit log over MCP for agent-side introspection.

This turns `homn` from a black-box guard into a **queryable peer**. The agent can reason about its own constraints. See [ADR-0006](adr/0006-mcp-server.md) for the deeper rationale.

## The hook integration (where the daemon gets fed)

Two paths, in priority order:

### Path A — Hook (primary)

`PermissionRequest` hook in `~/.claude/settings.json` calls `homn hook permission-request`. The hook subcommand POSTs the payload to the daemon socket and writes the daemon's response in Claude's expected hook return format.

**Caveat**: Anthropic bug [#19298](https://github.com/anthropics/claude-code/issues/19298) — *deny* returns from this hook are currently ignored. *Allow* works. We treat the hook as authoritative for *allow*, fallback for *deny*.

### Path B — PTY-tap wrapper (fallback for deny)

`homn run claude ...` spawns `claude` with a PTY. The wrapper:

1. Plumbs Claude's stdout to the user's terminal (read-only tap, no rewrite).
2. Regex-matches the permission prompt pattern.
3. Posts the prompt to the daemon over the socket in parallel.
4. If the daemon returns `deny` before the user types `y`, the wrapper writes `n\n` to Claude's stdin (synthesized as if the user typed it).
5. If the daemon returns `allow`, the wrapper writes `y\n`.
6. If the daemon takes too long, the user's interactive prompt is still there as a fallback — no decision is silent.

This is opt-in. Users who trust the hook path can skip the PTY tax. See [ADR-0003](adr/0003-pty-fallback.md).

## TUI prompt (the v1 default surface)

Before the Tauri face exists, *ask* decisions render directly in the calling terminal:

```
═══ homn: permission request ═══════════════════
session: cloakpipe-refactor
tool:    Bash
command: git push origin main
cwd:     ~/dev/cloakpipe
context: [[wiki/concepts/release-process]] — see §branch-protection

[a]llow  [d]eny  [A]lways allow  [D]always deny  [s]how rule that would be created
> _
```

Hotkeys map to surface answers; capital letters trigger the learning subsystem.

The TUI prompt is rendered by `homn-tui` using `ratatui`. It's the only surface that ships in Phase 1; the face arrives in Phase 2.

## Remote approval (ntfy mirror)

Optional. Configure a topic, and when *ask* fires *and* the user is idle ≥N minutes, the prompt mirrors to phone via ntfy. The phone push includes action buttons (allow / deny / always-allow) that resolve the decision via a callback.

Not a differentiator (every existing tool does this), but parity matters.

## Sync (open-core paid tier)

- **Free**: rules are local files. Version them in your own dotfiles repo.
- **Paid (team plan)**: hosted sync service that distributes signed rule files across team members' machines. Verified at install. Modeled on `atuin`'s sync — same encryption, same TOS.

This is the **only** paid feature in v1. Everything else is OSS.

## Failure modes

- **Daemon crashes mid-decision**: hook falls through to Claude's default (`ask` shown in terminal). Wrapper falls back to letting Claude's prompt show. Graceful.
- **Policy file has a syntax error**: daemon logs the error, loads the last-good version, surfaces a *learning suggestion* (sic) to the user via TUI. Never crashes.
- **Rhai rule runs forever**: 50ms wall-clock budget per rule, enforced by `Engine::set_max_operations`. Exceed → log + default to `ask`. See [risks/known-unknowns.md](../risks/known-unknowns.md).
- **SQLite locked**: audit write retries with backoff (audit is "best-effort soon", not synchronous).

## Phase 1 exit criteria

Detailed in [phases/phase-1-policy.md](../phases/phase-1-policy.md). Top three:

1. Daemon runs for ≥7 days on the author's machine with no crashes; audit log shows ≥1000 decisions.
2. `homn log --denied --since 7d` is something the author actually reads weekly.
3. ≥3 external users have written custom rule files and report they work as expected.
