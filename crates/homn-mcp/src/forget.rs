//! The `forget` MCP tool backing (US4 / T047) — the one WRITE tool. The trait is the seam; the
//! daemon-side `homnd::store::Store::forget` (agidb unlearn) backs it, and the daemon emits a
//! `DeletionReceipt` to the audit ledger. After success, the matched memory stops surfacing in
//! the other six (read) tools. Read-path tools are egress-free; this is the sole write surface.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use homn_types::{DeletionReceipt, ForgetScope};

/// Anything that can `forget` a scope (the unlearn primitive). The MCP `forget` tool calls this
/// and returns the receipt id + match_count.
#[async_trait]
pub trait Forget: Send + Sync {
    /// Forget memories matching `scope`, returning a [`DeletionReceipt`] (proves the scope
    /// without re-exposing content — Invariant 3 / FR-024).
    async fn forget(&self, scope: &ForgetScope) -> anyhow::Result<DeletionReceipt>;
}

/// In-process `Forget` over a closure-like canned receipt — for tests.
pub struct MemoryForget {
    receipt: Mutex<homn_types::DeletionReceipt>,
}

use std::sync::Mutex;

impl MemoryForget {
    /// A `MemoryForget` that always returns this receipt (match_count fixed at construction).
    pub fn returning(receipt: homn_types::DeletionReceipt) -> Self {
        Self {
            receipt: Mutex::new(receipt),
        }
    }
}

#[async_trait]
impl Forget for MemoryForget {
    async fn forget(&self, _scope: &ForgetScope) -> anyhow::Result<DeletionReceipt> {
        Ok(self.receipt.lock().expect("lock poisoned").clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn memory_forget_returns_the_canned_receipt() {
        let r = DeletionReceipt {
            scope: ForgetScope::Pattern("pricing".into()),
            match_count: 3,
            at: Utc::now(),
        };
        let f = MemoryForget::returning(r.clone());
        let got = f.forget(&ForgetScope::Entity("x".into())).await.unwrap();
        assert_eq!(got.match_count, 3);
        assert!(matches!(got.scope, ForgetScope::Pattern(_)));
    }
}
