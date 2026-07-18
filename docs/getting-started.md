# Getting started — homn alpha

> 5-minute walkthrough. Targets: Linux + macOS. Windows comes later.

## Quick start

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

That installs the binary, seeds a policy, wires the Claude Code hook, and starts the
daemon. The rest of this page covers the manual, step-by-step path and how each piece works.

## What works today

- **Deterministic deny** via Claude Code's `PermissionRequest` hook + the **`homn run claude`** PTY wrapper (which enforces deny even with upstream bug [#19298](https://github.com/anthropics/claude-code/issues/19298) by synthesizing `n\n` into the prompt when the audit log shows a recent deny).
- **Interactive ask path**: when a rule says `ask`, the hook opens an inline TUI prompt on `/dev/tty` with the tool, input preview, rule citation, and `a`/`d`/`A`/`D` hotkeys.
- **Audit log**: every decision (allow / deny / ask) lands in SQLite with the rule that fired and the latency. Queryable via `homn log` with filters.
- **Hot-reload**: editing `~/.config/homn/policies/default.rhai` is picked up within ~50ms without restarting the daemon. A syntactically broken edit keeps the previous ruleset active.
- **MCP server** (`homn mcp stdio`): the agent itself can query `query_policy`, `explain_decision`, `recent_decisions` — the novelty of this whole project.

## What doesn't work yet

- **Learning subsystem** (T060–T068): after 5 consistent same-answer asks, `homn` will suggest promoting to a rule. Not built yet.
- **The face** (Phase 2): the always-on-top ASCII window. Default OFF anyway.
- **`ctxgraph` integration** (Phase 3): session resumption, open-loop nudges, context-aware policy rules.

## Prerequisites

- Rust stable (1.88+) — `rustup default stable`.
- Claude Code installed and working.
- `~/.config` and `~/.local/share` writable (standard XDG).

## Step 1 — Build + install the binary

```sh
git clone https://github.com/rohansx/homn.git
cd homn
cargo build --release
sudo install -m 0755 target/release/homn /usr/local/bin/homn
# or, if /usr/local isn't your style:
cargo install --path crates/homn-bin
```

Verify:

```sh
homn --version
# expected: homn 0.1.0
```

## Step 2 — Wire the hook into Claude Code

```sh
homn install --apply
```

This:

1. Reads `~/.claude/settings.json` (or creates it if missing).
2. Merges in a `PermissionRequest` hook pointing at `homn hook permission-request`.
3. Writes a timestamped backup of the original first.
4. Is idempotent — safe to re-run.

Verify with:

```sh
homn install
# prints the snippet without modifying anything; useful to inspect what was added
```

## Step 3 — Write a policy

The repo ships a starter ruleset at [`policies/default.rhai`](../policies/default.rhai), plus
two alternative profiles — [`strict.rhai`](../policies/strict.rhai) (locked down) and
[`relaxed.rhai`](../policies/relaxed.rhai) (trusted projects). Copy whichever fits:

```sh
mkdir -p ~/.config/homn/policies
cp policies/default.rhai ~/.config/homn/policies/default.rhai
```

Or just run `homn rule edit` — it seeds `default.rhai` for you and opens it in `$EDITOR`.

Or write your own from scratch — the DSL is documented in [`docs/technical/policy-language.md`](technical/policy-language.md).

A rule is one line:

```rhai
<verb> if <expression>;
```

Verbs: `allow`, `deny`, `ask`. Expressions are Rhai boolean expressions with access to `tool`, `cmd`, `path`, `url`, `cwd`, `home`, `session_id`, plus helpers `matches` (glob) and `regex`.

```rhai
deny  if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny  if tool == "Bash" && cmd.matches("git push --force *");
allow if tool == "Read" && path.starts_with(home);
allow if tool == "Bash" && cmd.matches("npm run *");
ask   if tool == "Bash" && cmd.matches("git push * main");   // for now, this means "let Claude prompt"
```

Evaluation order is deny → ask → allow; first matching rule wins. No match → default ask
— so you never need an explicit `ask if true` catch-all (and shouldn't add one: asks run
before allows, so a catch-all `ask` would shadow every `allow` rule). See [docs/architecture/policy-engine.md](architecture/policy-engine.md) for the rationale.

## Step 4 — Run the daemon

```sh
homn daemon --foreground
```

You'll see:

```
INFO homn: starting daemon foreground=true socket=/run/user/1000/homn.sock
INFO homn_daemon: loading default policy path=/home/you/.config/homn/policies/default.rhai
INFO homn_daemon: opening audit DB path=/home/you/.local/share/homn/audit.db
INFO homn_daemon: homn daemon listening socket=/run/user/1000/homn.sock
```

### As a systemd user service (Linux)

```sh
mkdir -p ~/.config/systemd/user
cp dist/homn.service ~/.config/systemd/user/homn.service
sed -i "s|%h/.cargo/bin/homn|$(which homn)|" ~/.config/systemd/user/homn.service
systemctl --user daemon-reload
systemctl --user enable --now homn
systemctl --user status homn
```

See [`dist/README.md`](../dist/README.md) for the full unit explanation (resource limits,
network sandboxing, filesystem allowlist).

macOS launchd plist coming in a future polish slice.

## Step 5 — Use Claude normally

```sh
claude
```

Have it try a destructive thing in a non-`/tmp` directory:

> *"Run `rm -rf ~/old-projects`"*

You should see the call denied (Claude reports the tool was blocked). Verify:

```sh
homn log --denied --since 1m
```

You'll see a row with the rule that fired and the path Claude tried.

## Step 6 — Read your history

```sh
# Tail
homn log

# Filter
homn log --denied
homn log --allowed --tool Read
homn log --since 24h --grep "git push"
homn log --asked

# Scripting
homn log --denied --json | jq '.tool_input.command'
```

Colors render when stdout is a TTY; piping gives clean output.

## Step 7 — Iterate your policy

Just edit the file. The daemon picks up changes via `inotify` within ~50 ms; no restart needed.

```sh
$EDITOR ~/.config/homn/policies/default.rhai
# That's it — the daemon log will show `policy hot-reloaded deny=3 ask=4 allow=12` or similar.
```

A syntactically broken edit (e.g. unterminated string) is logged as a warning; the previously-loaded ruleset stays active until you fix it. You'll never accidentally empty out your policy.

## Step 7b (optional) — Let Claude query its own constraints via MCP

This is the novel part. Configure `~/.claude.json`:

```jsonc
{
  "mcpServers": {
    "homn": {
      "command": "homn",
      "args": ["mcp", "stdio"]
    }
  }
}
```

Now Claude has three new tools available:

- **`query_policy`** — dry-run a tool call against your rules. Use `before` attempting an action you suspect may be denied. Returns the decision + rule that would fire. Doesn't log to audit.
- **`explain_decision`** — look up `decision_id N` and see the rule that fired. Use after a deny so you can propose an alternative.
- **`recent_decisions`** — tail the audit. Filterable by tool, decision, FTS-search.

Test it:

```
You: "What would happen if you tried `rm -rf ~/Documents`?"
Claude: (calls query_policy) "Your policy would deny this — rule default.rhai:2 says 
        `deny if tool == \"Bash\" && cmd.contains(\"rm -rf\")`. Want me to use trash-cli instead?"
```

## Step 7c (optional) — Use the PTY-tap wrapper

The daemon's deny return through the hook is best-effort because of Anthropic bug [#19298](https://github.com/anthropics/claude-code/issues/19298) — Claude may still show the prompt. `homn run claude` wraps Claude in a PTY and synthesizes `n\n` into the prompt when the audit shows a recent deny.

```sh
homn run claude                        # use this instead of plain `claude`
```

Tradeoff: harder-to-debug terminal multiplexing. Worth it if you care about hard-deny semantics.

## Step 8 — Uninstall

```sh
# Remove the hook entry (manual: edit ~/.claude/settings.json and remove the `_homn` block)
# Or restore your backup:
cp ~/.claude/settings.json.bak.* ~/.claude/settings.json   # pick the most recent
```

## Common issues

| Symptom | Fix |
|---|---|
| `homn install --apply` says "already installed" but you don't see the hook | Check `~/.claude/settings.json` — look for `_homn` marker. May be present already from a prior install. |
| `homn log` returns nothing | Daemon hasn't seen any decisions yet, or the audit DB is elsewhere. Check `homn.toml` (or defaults to `~/.local/share/homn/audit.db`). |
| Daemon log says "no homn.toml found; using defaults" | Fine — that's the expected default path. Create one only if you want to override defaults. |
| Claude still shows the prompt even though you have a deny rule | This is Anthropic bug [#19298](https://github.com/anthropics/claude-code/issues/19298). The PTY wrapper (T053–T057) is the workaround; it ships in the next slice. |

## Next steps

- Read [`docs/architecture/overview.md`](architecture/overview.md) for the three-layer system design.
- Read [`docs/risks/known-unknowns.md`](risks/known-unknowns.md) for what could still break.
- Open an issue if a rule that should work doesn't — include the relevant `homn log --json` row.
