# ADR-0002 — Rust + Rhai for the daemon and rules engine

**Status**: Accepted

## Context

The daemon needs to be:

- Long-lived (24/7 user-session process)
- Fast (sub-millisecond policy eval; tool calls fire many times per minute during heavy agent use)
- Trivially installable (one binary)
- Easy to expose as MCP (the `rmcp` reference implementation is Rust)
- Compatible with the existing `ctxgraph` codebase (already Rust)

The rules DSL needs to be:

- Embedded in the daemon (no shelling out per evaluation)
- Sandboxed (user-authored, can't escape into shell, can't hang forever)
- Readable by humans who aren't programmers (the user is the author)
- Capable of basic logic (string matching, path predicates, comparisons)

## Decision

**Language**: Rust for everything.

**Rules engine**: [Rhai](https://rhai.rs) (rust-native embedded scripting).

### Rejected alternatives

| Alternative          | Reason rejected                                                    |
|----------------------|--------------------------------------------------------------------|
| Go for the daemon    | Slower MCP ecosystem; no `ctxgraph` reuse; larger memory footprint |
| Python for the daemon| Cold start unacceptable for hook latency; deployment is harder     |
| Node for the daemon  | Memory characteristics worse than Rust; deployment is harder       |
| Starlark for rules   | Fewer Rust integrations; more pythonic syntax than the author wants|
| Lua for rules        | `mlua` is good but Rhai is rust-native and the ecosystem is enough |
| Native DSL           | Author wanted shipping over invented-here                          |
| TOML / YAML config   | Can't express `cmd.matches("npm run *") && !cwd.starts_with(...)`  |

### Why Rhai specifically

- Pure Rust, no `unsafe`.
- Sandboxing primitives: `set_max_operations`, `set_max_call_levels`, `set_max_string_size` — all enforced via the embedded engine, not via OS-level isolation.
- Expression-oriented syntax that reads like a constraints file:
  ```rhai
  allow if tool == "Read" && path.starts_with(home);
  ```
- Good integration story with Rust: closures, custom types, function modules — we expose `path`, `cmd`, `cwd`, `ctxgraph.*` as native Rust functions callable from Rhai.

## Consequences

- Every rule eval has a wall-clock budget (50ms default, configurable). Enforced via `Engine::set_max_operations`. Exceed → log + default to `ask`. See [risks/known-unknowns.md](../../risks/known-unknowns.md).
- Users who'd prefer Starlark have to wait for a v2 alternative engine — we don't commit to one in v1.
- Documentation for the policy DSL is its own surface — see [technical/policy-language.md](../../technical/policy-language.md).
- We accept a smaller Rhai community vs Lua/Python as a tradeoff for the install-and-go story.
