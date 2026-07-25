# homn v2 — "the local human" doc set

The pivot from "policy engine for coding agents" to a local ambient memory + clone daemon, absorbing homn v1 as the governance layer. Written 2026-07-17, sourced from the July 2026 pivot plan plus a verified audit of every dependency repo.

Read in order:

1. [`product-overview.md`](./product-overview.md) — what homn becomes, the wedge, positioning, what carries over from v1
2. [`architecture.md`](./architecture.md) — five layers, data flow, component decisions, local/cloud split, data model, MCP surface, invariants
3. [`tech-plan.md`](./tech-plan.md) — Phase 0 validation gate through Phase 5 body, asset audit, risks, definition of done
4. [`l5-body-clicky-adoption.md`](./l5-body-clicky-adoption.md) — Phase 5 detail: adopt Clicky (ClickyX, MIT, Rust+Tauri) as the body — the three wiring points (memory-in, hands→gate, voice-out), the moat, the overlap de-dupe, the retire-homn-face call. Decision recorded in [ADR-0008](../architecture/adr/0008-clicky-as-l5-body.md).

Supersedes: `docs/product/overview.md`, `docs/phases/phase-2-face.md`, `docs/phases/phase-3-brain.md`, `docs/phases/milestones.md`. Phase 1 v1 (`specs/001-policy-engine/`) remains the record of the shipped policy engine.
