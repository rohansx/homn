//! MCP server for `homn`.
//!
//! Exposes `query_policy`, `explain_decision`, `suggest_rule`, `recent_decisions` (and proxies
//! ctxgraph tools in Phase 3). Lets the agent introspect its own constraints — the most novel
//! piece of the product. See [ADR-0006](../../../docs/architecture/adr/0006-mcp-server.md).
//!
//! Implementation lands across T073–T079.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
