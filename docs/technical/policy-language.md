# Technical — Policy language (Rhai DSL)

> The user-facing rule DSL. Rhai under the hood, but the surface area is a deliberately small subset.

## File layout

```
$XDG_CONFIG_HOME/homn/policies/
├── default.rhai              # baseline rules, applied to every session
├── <repo-slug>.rhai          # project-scoped overrides (matched by cwd)
└── ignored/                  # learning suggestions the user rejected
```

The daemon picks files in order:
1. `default.rhai` (always)
2. `<repo-slug>.rhai` if the calling session's cwd starts with a known project root

Rules from project files **stack on top** of default — they don't replace it. To override a default, write a more specific rule.

## Rule syntax

```rhai
<verb> if <expr>;
```

`<verb>` is one of `allow`, `deny`, `ask`. `<expr>` is a Rhai boolean expression that has access to the request context.

```rhai
allow if tool == "Read" && path.starts_with(home);
deny  if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
ask   if tool == "Bash" && cmd.matches("git push * main");
```

## Available context variables

| Name             | Type   | Meaning                                                |
|------------------|--------|--------------------------------------------------------|
| `tool`           | string | e.g. `"Bash"`, `"Read"`, `"WebFetch"`, `"mcp__supabase__query"` |
| `tool_input`     | object | Full tool payload (varies per tool)                    |
| `cmd`            | string | For Bash: the command. For others: empty.              |
| `path`           | string | For Read/Edit/Write: the file path. For others: empty. |
| `url`            | string | For WebFetch: the URL. For others: empty.              |
| `cwd`            | string | Current working directory of the session              |
| `session_id`     | string | Stable per Claude session                              |
| `home`           | string | `$HOME`                                                |
| `now`            | int    | Unix epoch seconds                                     |
| `weekday`        | int    | 0=Sunday, 6=Saturday                                   |
| `hour`           | int    | 0–23, local time                                       |

## String helpers (Rhai-builtin + ours)

| Method                                  | Description                                  |
|-----------------------------------------|----------------------------------------------|
| `starts_with(s)` / `ends_with(s)`       | Standard                                     |
| `contains(s)`                           | Substring match                              |
| `matches(pattern)`                      | Glob-style with `*` and `?`                  |
| `regex(pattern)`                        | RE2 regex match (no backtracking, safe)      |
| `split(sep)`                            | Returns array                                |
| `lower()` / `upper()`                   | Case folding                                 |

`matches` is the recommended one for command patterns — glob is enough for 95% of cases and is faster than regex.

## ctxgraph helpers (layer 3 only)

Only available when ctxgraph is configured. If ctxgraph is offline or hasn't ingested the relevant data, these return `()` (Rhai's unit, which is falsy in `if` context) — never block, never throw.

| Helper                                              | Returns                                           |
|-----------------------------------------------------|---------------------------------------------------|
| `ctxgraph.recently_edited(path, hours)`             | bool: did *I* edit this file in the last N hours? |
| `ctxgraph.has_open_pr(cwd, branch)`                 | bool                                              |
| `ctxgraph.has_open_pr_passing_ci(cwd, branch)`      | bool                                              |
| `ctxgraph.last_commit_age_minutes(cwd)`             | int                                               |
| `ctxgraph.matches_wiki_tag(string, tag)`            | bool                                              |
| `ctxgraph.previous_decisions(tool, input_hash)`     | int (count of prior decisions for this exact call)|

## Evaluation semantics

1. Rules are evaluated in order: **deny → ask → allow**. First matching rule wins.
2. Within a verb, rules are tried in file order (default first, then project file).
3. If no rule matches: implicit `ask` (the "default: ask but learn" line at the bottom of `default.rhai`).
4. Each rule has a hard 50ms wall-clock budget. Exceed → log + treat as non-match.
5. Per-call total budget: 200ms across all rules. Exceed → log + return `ask`.

## Sandboxing limits (Rhai engine config)

```rust
engine.set_max_operations(100_000);     // ~50ms on commodity hardware
engine.set_max_call_levels(32);
engine.set_max_string_size(8 * 1024);
engine.set_max_array_size(1024);
engine.set_max_modules(8);
engine.set_max_expr_depths(64, 64);
// no file I/O, no network, no shell — Rhai's defaults
```

These are enforced in the Rhai engine itself, not the OS. They cannot be defeated by user-authored rules.

## Learning-generated rules

When the user accepts a learning suggestion, `homn` appends a rule like:

```rhai
// added by homn learning on 2026-05-13 — 7 consistent allows for this pattern
// session ids: 01HXY1..., 01HXY2..., ... (see audit log)
allow if tool == "Bash" && cmd.matches("git push origin feat/*");
```

The user can edit or delete the rule. `homn` never auto-modifies a rule once written — only appends new ones with comments.

## Errors and reload

- Syntax errors in any policy file: daemon logs the error, falls back to the last-good version of *that file*, surfaces a learning notification ("policy file X has an error since 14:23 — rules from it are not active").
- File changes are picked up via `inotify` / `kqueue`; reload is hot (no daemon restart).
- Reload is atomic — either the new file parses cleanly and replaces the in-memory rules, or the old rules stay active.

## Example: a realistic `default.rhai`

```rhai
// reads inside home are fine
allow if tool == "Read" && path.starts_with(home);
allow if tool == "Read" && path.starts_with("/etc");
allow if tool == "Read" && path.starts_with("/usr");

// common dev tooling
allow if tool == "Bash" && cmd.matches("ls *");
allow if tool == "Bash" && cmd.matches("cat *");
allow if tool == "Bash" && cmd.matches("grep *");
allow if tool == "Bash" && cmd.matches("npm run *");
allow if tool == "Bash" && cmd.matches("cargo (build|test|check|clippy) *");
allow if tool == "Bash" && cmd.matches("pytest *");
allow if tool == "Bash" && cmd.regex("^git (status|log|diff|branch|fetch)( |$)");

// hard denies
deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny if tool == "Bash" && cmd.matches("git push --force *");
deny if tool == "Bash" && cmd.contains(":(){ :|:& };:");          // fork bomb
deny if tool == "WebFetch" && url.contains("169.254.169.254");    // cloud metadata
deny if tool == "Read"  && path.contains("/.ssh/id_");            // private keys
deny if tool == "Read"  && path.contains("/.aws/credentials");

// asks
ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "Bash" && cmd.matches("git push * master");
ask if tool == "Bash" && cmd.contains("sudo");
ask if tool == "WebFetch" && url.contains("internal.");

// default: ask + learn
ask if true;
```
