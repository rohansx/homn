# Getting started — homn alpha

> 5-minute walkthrough. Targets: Linux + macOS. Windows comes later.

## What works today (v0 alpha)

- **Deterministic deny** via Claude Code's `PermissionRequest` hook. When a rule in your policy file matches with verb `deny`, the daemon returns `{"behavior": "deny"}` and Claude does not run the call.
- **Audit log**: every decision (allow / deny / ask) lands in SQLite with the rule that fired and the latency. Queryable via `homn log`.
- **Hot path**: rule eval ≤50 ms p95 on a starter ruleset; audit write off the hot path.

## What doesn't work yet (deferred to later phases)

- **TUI `ask` round-trip** (T031–T033): if a rule says `ask`, the daemon currently returns `ask` to Claude, which then shows its own interactive prompt. The dedicated TUI prompt that blocks on the daemon side lands in the next slice.
- **PTY-tap wrapper** (T053–T057): `homn run claude` will guarantee deny even if Anthropic upstream bug [#19298](https://github.com/anthropics/claude-code/issues/19298) is still affecting your version of Claude Code. Right now the hook return is best-effort for deny.
- **The face** (Phase 2): the always-on-top ASCII window. Default OFF anyway.
- **`ctxgraph` integration** (Phase 3): session resumption, open-loop nudges, context-aware policy rules.

## Prerequisites

- Rust stable (1.83+) — `rustup default stable`.
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

The repo ships a starter ruleset at [`policies/example.rhai`](../policies/example.rhai). Copy it:

```sh
mkdir -p ~/.config/homn/policies
cp policies/example.rhai ~/.config/homn/policies/default.rhai
```

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
ask   if true;                                                // default catch-all
```

Evaluation order is deny → ask → allow; first matching rule wins. No match → default ask. See [docs/architecture/policy-engine.md](architecture/policy-engine.md) for the rationale.

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

Edit `~/.config/homn/policies/default.rhai` and **restart the daemon** to pick up changes. Hot reload via inotify is T026 — coming soon.

```sh
$EDITOR ~/.config/homn/policies/default.rhai
# Ctrl-C the daemon, restart it
```

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
