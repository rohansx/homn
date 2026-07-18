//! Shared types for the `homn` workspace.
//!
//! These types are the wire-level contract between daemon, hook, surfaces, and MCP server.
//! See [`docs/architecture/data-flow.md`](../../../docs/architecture/data-flow.md) and
//! [`specs/001-policy-engine/data-model.md`](../../../specs/001-policy-engine/data-model.md).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bus;
pub mod decision;
pub mod rpc;
pub mod session;

// v2 ambient-memory types (spec 002).
pub mod observation;
pub mod receipt;
pub mod redaction;
pub mod source;

pub use bus::{BusEvent, HighStakesKind};
pub use decision::{
    Decision, DecisionContext, DecisionRecord, DecisionSource, HumanAnswer, RuleSourceLocation,
    Surface,
};
pub use rpc::{ErrorObject, Request, Response, RpcError};
pub use session::{Session, SessionId, SessionKind};

// v2 ambient-memory re-exports.
pub use observation::{Observation, Provenance, SpeakerTag};
pub use receipt::{
    DecisionReceipt, DeletionReceipt, DisclosureReceipt, ForgetScope, IngestOutcome, Receipt,
    ReceiptId,
};
pub use redaction::{RedactionKind, RedactionRef, SpanRef};
pub use source::{Cursor, RawCapture, SourceKind, Watermark};
