# Architecture Decision Records

> Each ADR captures one architectural commitment, the alternatives considered, and why we chose this one. Format follows Michael Nygard's classic structure: Status / Context / Decision / Consequences.

## Index

| #     | Title                                          | Status     |
|-------|------------------------------------------------|------------|
| 0001  | [Name the project `homn`](0001-naming.md)        | accepted   |
| 0002  | [Rust + Rhai for the daemon and rules engine](0002-rust-rhai.md) | accepted   |
| 0003  | [PTY-tap fallback for deny path](0003-pty-fallback.md) | accepted   |
| 0004  | [Tauri over egui for the face](0004-tauri-vs-egui.md) | accepted   |
| 0005  | [`ctxgraph` stays a separate repo](0005-ctxgraph-separate.md) | accepted   |
| 0006  | [Expose `homn` as an MCP server](0006-mcp-server.md) | accepted   |

## Conventions

- Numbered sequentially.
- One file per decision.
- "Accepted" once a decision is final. Use "Superseded by ADR-NNNN" when revisiting.
- Keep ADRs short (≤300 words ideal, ≤600 hard ceiling). The codebase is the long-form documentation; ADRs are the *why*.
