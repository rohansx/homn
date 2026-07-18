//! T036 — hash-chain integrity for the receipt ledger.
//!
//! The chain must verify end-to-end, tampering with any row must break verification
//! from that row on, and no ledger row may ever contain plaintext of redacted content.

use chrono::{DateTime, Utc};
use homn_audit::Db;
use homn_types::{
    DecisionReceipt, DeletionReceipt, DisclosureReceipt, ForgetScope, IngestOutcome, Receipt,
};

fn ts(secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

/// A distinct decision receipt per index, so tampering targets are unambiguous.
fn decision(i: i64) -> Receipt {
    Receipt::Decision(DecisionReceipt {
        outcome: IngestOutcome::Redact,
        policy_id: Some(format!("rule-{i}")),
        observation_ref: Some(format!("01OBS{i}")),
        at: ts(i),
    })
}

#[tokio::test]
async fn chain_verifies_end_to_end() {
    let db = Db::in_memory().await.unwrap();
    for i in 0..4 {
        db.append_receipt(&decision(i)).await.unwrap();
    }
    db.append_receipt(&Receipt::Disclosure(DisclosureReceipt {
        policy_id: "allow-cloud".into(),
        model: "claude-haiku".into(),
        payload_digest: "b3:deadbeef".into(),
        at: ts(10),
    }))
    .await
    .unwrap();
    db.append_receipt(&Receipt::Deletion(DeletionReceipt {
        scope: ForgetScope::Entity("Acme".into()),
        match_count: 2,
        at: ts(11),
    }))
    .await
    .unwrap();

    let v = db.verify_ledger().await.unwrap();
    assert_eq!(v.total, 6);
    assert_eq!(v.first_bad_seq, None);
    assert!(v.is_valid(), "fresh chain must verify: {v:?}");

    // Adjacent rows actually link: each entry's prev_hash is the previous this_hash.
    let entries = db.ledger_tail(10).await.unwrap();
    assert_eq!(entries.len(), 6, "tail returns every row");
    // Newest first.
    assert!(entries[0].seq > entries[1].seq);
    for pair in entries.windows(2) {
        assert_eq!(pair[0].prev_hash, pair[1].this_hash, "chain link broken");
    }
}

#[tokio::test]
async fn empty_ledger_verifies_as_valid() {
    let db = Db::in_memory().await.unwrap();
    let v = db.verify_ledger().await.unwrap();
    assert_eq!(v.total, 0);
    assert!(v.is_valid());
}

#[tokio::test]
async fn tampering_a_row_breaks_verification_at_that_row() {
    let db = Db::in_memory().await.unwrap();
    for i in 0..5 {
        db.append_receipt(&decision(i)).await.unwrap();
    }
    // Attacker edits the stored receipt payload of row 3 without touching hashes.
    db.conn()
        .call(|c| {
            let n = c.execute(
                "UPDATE ledger SET receipt = replace(receipt, 'rule-2', 'rule-666') WHERE seq = 3",
                [],
            )?;
            assert_eq!(n, 1, "tamper target row must exist");
            Ok(())
        })
        .await
        .unwrap();

    let v = db.verify_ledger().await.unwrap();
    assert!(!v.is_valid());
    assert_eq!(
        v.first_bad_seq,
        Some(3),
        "verification breaks at the tampered row"
    );
}

#[tokio::test]
async fn tampering_a_stored_hash_breaks_verification_at_that_row() {
    let db = Db::in_memory().await.unwrap();
    for i in 0..5 {
        db.append_receipt(&decision(i)).await.unwrap();
    }
    db.conn()
        .call(|c| {
            c.execute(
                "UPDATE ledger SET this_hash = replace(this_hash, substr(this_hash, 1, 1),
                        CASE substr(this_hash, 1, 1) WHEN 'a' THEN 'b' ELSE 'a' END)
                 WHERE seq = 2",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let v = db.verify_ledger().await.unwrap();
    assert!(!v.is_valid());
    assert_eq!(v.first_bad_seq, Some(2));
}

#[tokio::test]
async fn deleting_a_row_breaks_the_link_at_the_next_row() {
    let db = Db::in_memory().await.unwrap();
    for i in 0..5 {
        db.append_receipt(&decision(i)).await.unwrap();
    }
    db.conn()
        .call(|c| {
            c.execute("DELETE FROM ledger WHERE seq = 3", [])?;
            Ok(())
        })
        .await
        .unwrap();

    let v = db.verify_ledger().await.unwrap();
    assert!(!v.is_valid());
    // Row 3 is gone; row 4's prev_hash no longer matches row 2's this_hash.
    assert_eq!(
        v.first_bad_seq,
        Some(4),
        "break surfaces at the row after the deletion"
    );
}

#[tokio::test]
async fn ledger_rows_contain_no_redacted_plaintext() {
    const SECRET: &str = "sk-live-SUPERSECRET-4242";
    let db = Db::in_memory().await.unwrap();

    // Receipts about content whose secret was redacted: they carry only ids,
    // refs, digests, kinds — never the secret itself.
    let receipts = [
        Receipt::Decision(DecisionReceipt {
            outcome: IngestOutcome::Redact,
            policy_id: Some("redact-api-keys".into()),
            observation_ref: Some("01OBSA".into()),
            at: ts(1),
        }),
        Receipt::Disclosure(DisclosureReceipt {
            policy_id: "allow-cloud".into(),
            model: "claude-haiku".into(),
            payload_digest: "b3:0f0f0f0f".into(), // digest of the *redacted* payload
            at: ts(2),
        }),
        Receipt::Deletion(DeletionReceipt {
            scope: ForgetScope::Pattern("sk-live-*".into()),
            match_count: 1,
            at: ts(3),
        }),
    ];
    for r in &receipts {
        db.append_receipt(r).await.unwrap();
    }

    // Dump every column of every ledger row; the secret must appear nowhere.
    let dump: String = db
        .conn()
        .call(|c| {
            let v: String = c.query_row(
                "SELECT COALESCE(group_concat(seq || receipt || prev_hash || this_hash, x'0a'), '')
                 FROM ledger",
                [],
                |r| r.get(0),
            )?;
            Ok(v)
        })
        .await
        .unwrap();
    assert!(!dump.is_empty());
    assert!(
        !dump.contains(SECRET),
        "ledger must never store redacted plaintext"
    );
}
