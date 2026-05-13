# ADR-0005 — `ctxgraph` stays a separate repository

**Status**: Accepted

## Context

Layer 3 uses `ctxgraph` (existing local-first context graph engine, separate Rust project) as its memory subsystem. Two reasonable structures:

1. **Vendor ctxgraph into `homn`** as a workspace crate (or git submodule), develop in lockstep.
2. **Keep ctxgraph as a separate repo + crate**, depend on it via crates.io or git+ref.

## Decision

`ctxgraph` stays a separate repo. `homn` depends on it as a normal Rust crate.

### Rationale

- `ctxgraph` has **standalone value** beyond `homn`. Other consumers (the author's own projects, eventually third parties) want it as a library, not as a piece of `homn`.
- Vendoring would entangle layer 3's release cadence with ctxgraph's internal velocity. Separate repos = independent versioning, independent release notes, independent test suites.
- The "ctxgraph case study" launch post (planned for Phase 3 GTM) only works if ctxgraph is its own visible artifact people can star.
- Schema migrations live with the schema owner. If `homn` needs new entity types (`session`, `command`, `open_loop`), those land in ctxgraph as a versioned schema bump, not in `homn`'s own DB. Discipline: `homn` never writes a custom table that ctxgraph should own.

### Rejected alternatives

| Alternative                          | Reason rejected                                                   |
|--------------------------------------|-------------------------------------------------------------------|
| Vendor ctxgraph as a workspace crate | Couples release cadence; hides ctxgraph as a product              |
| Inline ctxgraph's storage in `homn`  | Defeats the standalone library value; doubles maintenance         |
| Build `homn`-specific memory layer   | Reinvents ctxgraph's engine; orphan storage layer                 |
| Use a third-party memory tool (mem0, etc.) | Not local-first; not bi-temporal; not yours                |

## Consequences

- Before Phase 3 begins, run a **ctxgraph readiness audit**: confirm the current API surface supports `homn`'s ingestor pattern + Rhai-callable query helpers. If gaps exist, file issues against ctxgraph first and schedule them ahead of Phase 3 work. (Tracked in [phases/phase-3-brain.md](../../phases/phase-3-brain.md).)
- Schema extensions for `homn`'s use cases land in ctxgraph as versioned migrations. Existing ctxgraph users must not break — if they would, the schema lands behind a feature flag until adoption catches up.
- The MCP server `homn` exposes is the *union* of `homn`-native tools and a proxy to ctxgraph's existing MCP tools. We don't duplicate; we re-export.
- If ctxgraph is ever extracted into a separate maintainership, `homn`'s dependency is a stable version pin — not a vendor copy that drifts.
