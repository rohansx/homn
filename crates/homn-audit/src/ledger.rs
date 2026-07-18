//! Hash-chained receipt ledger (specs/002-ambient-memory, T040).
//!
//! Every [`Receipt`] appended here gets `this_hash = blake3(prev_hash || canonical_json)`,
//! making the ledger tamper-evident: editing, re-hashing, or deleting any row breaks
//! verification from that row on. Consumed by `homn-gate` (receipt emission) and
//! `homn-bin` (`homn ledger verify` / listing).
//!
//! Known limitation: truncating the *tail* of the chain is undetectable from the chain
//! alone — that needs an external anchor (row count / head hash kept elsewhere).

use homn_types::Receipt;

use crate::Db;

/// `prev_hash` of the first row: 32 zero bytes, hex-encoded.
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One row of the receipt ledger, deserialized.
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    /// Position in the chain (SQLite rowid; monotonically increasing).
    pub seq: i64,
    /// The receipt itself.
    pub receipt: Receipt,
    /// Hex blake3 hash of the previous row ([`GENESIS_HASH`]-equivalent zeros for the first).
    pub prev_hash: String,
    /// Hex `blake3(prev_hash || canonical_json(receipt))`.
    pub this_hash: String,
}

/// Result of walking the chain. Valid iff `first_bad_seq` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerVerification {
    /// Number of rows examined.
    pub total: u64,
    /// First row whose link or hash failed to verify, if any. All rows from here on
    /// are untrusted.
    pub first_bad_seq: Option<i64>,
}

impl LedgerVerification {
    /// True when every row verified.
    pub fn is_valid(&self) -> bool {
        self.first_bad_seq.is_none()
    }
}

/// `blake3(prev_hash_hex || receipt_json)`, hex-encoded.
fn chain_hash(prev_hash: &str, receipt_json: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(prev_hash.as_bytes());
    h.update(receipt_json.as_bytes());
    h.finalize().to_hex().to_string()
}

/// Canonical JSON for hashing: serde field order is declaration order, which is stable
/// for our own types — no extra canonicalization pass needed.
fn canonical_json(receipt: &Receipt) -> anyhow::Result<String> {
    Ok(serde_json::to_string(receipt)?)
}

impl Db {
    /// Append a receipt to the hash-chained ledger, returning the persisted entry.
    ///
    /// Reading the chain head and inserting happen in one transaction, so concurrent
    /// appends can never fork the chain.
    pub async fn append_receipt(&self, receipt: &Receipt) -> anyhow::Result<LedgerEntry> {
        let json = canonical_json(receipt)?;
        let receipt = receipt.clone();
        let (seq, prev_hash, this_hash) = self
            .conn
            .call(move |c| {
                let tx = c.transaction()?;
                let prev_hash: String = tx.query_row(
                    "SELECT COALESCE(
                        (SELECT this_hash FROM ledger ORDER BY seq DESC LIMIT 1), ?)",
                    [GENESIS_HASH],
                    |r| r.get(0),
                )?;
                let this_hash = chain_hash(&prev_hash, &json);
                tx.execute(
                    "INSERT INTO ledger (receipt, prev_hash, this_hash) VALUES (?, ?, ?)",
                    rusqlite::params![json, prev_hash, this_hash],
                )?;
                let seq = tx.last_insert_rowid();
                tx.commit()?;
                Ok((seq, prev_hash, this_hash))
            })
            .await?;
        Ok(LedgerEntry {
            seq,
            receipt,
            prev_hash,
            this_hash,
        })
    }

    /// Walk the whole chain in order, recomputing every hash and link.
    ///
    /// Reports the first row that fails either check (stored hash ≠ recomputed hash, or
    /// `prev_hash` ≠ previous row's `this_hash`); everything from that row on is untrusted.
    pub async fn verify_ledger(&self) -> anyhow::Result<LedgerVerification> {
        let rows: Vec<(i64, String, String, String)> = self
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT seq, receipt, prev_hash, this_hash FROM ledger ORDER BY seq ASC",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;

        let total = rows.len() as u64;
        let mut expected_prev = GENESIS_HASH.to_owned();
        for (seq, receipt_json, prev_hash, this_hash) in rows {
            if prev_hash != expected_prev || chain_hash(&prev_hash, &receipt_json) != this_hash {
                return Ok(LedgerVerification {
                    total,
                    first_bad_seq: Some(seq),
                });
            }
            expected_prev = this_hash;
        }
        Ok(LedgerVerification {
            total,
            first_bad_seq: None,
        })
    }

    /// Return the most recent `limit` ledger entries, newest first (for `homn ledger`).
    ///
    /// Fails closed: a row whose receipt no longer parses is an error, not a skip.
    pub async fn ledger_tail(&self, limit: u32) -> anyhow::Result<Vec<LedgerEntry>> {
        let rows: Vec<(i64, String, String, String)> = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT seq, receipt, prev_hash, this_hash FROM ledger
                     ORDER BY seq DESC LIMIT ?",
                )?;
                let rows = stmt
                    .query_map([limit.max(1) as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        rows.into_iter()
            .map(|(seq, receipt_json, prev_hash, this_hash)| {
                let receipt: Receipt = serde_json::from_str(&receipt_json)
                    .map_err(|e| anyhow::anyhow!("ledger row {seq}: unparseable receipt: {e}"))?;
                Ok(LedgerEntry {
                    seq,
                    receipt,
                    prev_hash,
                    this_hash,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_hash_is_deterministic_and_link_sensitive() {
        let a = chain_hash(GENESIS_HASH, r#"{"type":"decision"}"#);
        let b = chain_hash(GENESIS_HASH, r#"{"type":"decision"}"#);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        // Different prev_hash or payload ⇒ different hash.
        assert_ne!(a, chain_hash(&a, r#"{"type":"decision"}"#));
        assert_ne!(a, chain_hash(GENESIS_HASH, r#"{"type":"deletion"}"#));
    }
}
