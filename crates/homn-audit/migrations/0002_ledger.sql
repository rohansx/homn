-- homn-audit migration 0002: hash-chained receipt ledger
--
-- See specs/002-ambient-memory/data-model.md §Receipts and contracts/gate-pipeline.md R-4.
-- Each row stores the canonical JSON of one Receipt (decision | disclosure | deletion) plus
-- its blake3 chain link: this_hash = blake3(prev_hash || receipt). Receipts are plaintext-free
-- by construction (Invariant 3 / FR-015); this table adds no column that could hold content.

CREATE TABLE IF NOT EXISTS ledger (
  seq       INTEGER PRIMARY KEY AUTOINCREMENT,
  receipt   TEXT NOT NULL,   -- canonical JSON of homn_types::Receipt
  prev_hash TEXT NOT NULL,   -- hex blake3 of the previous row's this_hash (genesis: 64 zeros)
  this_hash TEXT NOT NULL    -- hex blake3(prev_hash || receipt)
);
