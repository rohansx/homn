//! T045 — a `forget` writes a chained DeletionReceipt (scope, match_count, at)
//! containing none of the forgotten plaintext (Invariant 3, FR-024).

use chrono::{DateTime, Utc};
use homn_audit::Db;
use homn_types::{DecisionReceipt, DeletionReceipt, ForgetScope, IngestOutcome, Receipt};

/// Plaintext of the memories being forgotten — must never reach the ledger.
const FORGOTTEN: &str = "the Falcon launch is delayed to Q3, per the private standup";

fn ts(secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

#[tokio::test]
async fn forget_writes_a_chained_deletion_receipt() {
    let db = Db::in_memory().await.unwrap();

    // Prior ledger activity, so the deletion receipt has a real link to chain onto.
    db.append_receipt(&Receipt::Decision(DecisionReceipt {
        outcome: IngestOutcome::Allow,
        policy_id: None,
        observation_ref: Some("01OBSX".into()),
        at: ts(0),
    }))
    .await
    .unwrap();

    // The forget: 3 memories about "Project Falcon" removed. The receipt records
    // scope + match_count + at — proof of what was removed, not the content.
    let deletion = Receipt::Deletion(DeletionReceipt {
        scope: ForgetScope::Entity("Project Falcon".into()),
        match_count: 3,
        at: ts(60),
    });
    let entry = db.append_receipt(&deletion).await.unwrap();

    // Chained: links to the prior row and the whole chain verifies.
    let prior = &db.ledger_tail(2).await.unwrap()[1];
    assert_eq!(
        entry.prev_hash, prior.this_hash,
        "deletion receipt must chain"
    );
    let v = db.verify_ledger().await.unwrap();
    assert_eq!(v.total, 2);
    assert!(v.is_valid());

    // Round-trips with scope, match_count, at intact.
    let got = &db.ledger_tail(1).await.unwrap()[0];
    assert_eq!(got.seq, entry.seq);
    match &got.receipt {
        Receipt::Deletion(d) => {
            assert_eq!(d.scope, ForgetScope::Entity("Project Falcon".into()));
            assert_eq!(d.match_count, 3);
            assert_eq!(d.at, ts(60));
        }
        other => panic!("expected a deletion receipt, got {other:?}"),
    }
}

#[tokio::test]
async fn deletion_receipt_contains_none_of_the_forgotten_plaintext() {
    let db = Db::in_memory().await.unwrap();
    db.append_receipt(&Receipt::Deletion(DeletionReceipt {
        scope: ForgetScope::Entity("Project Falcon".into()),
        match_count: 3,
        at: ts(60),
    }))
    .await
    .unwrap();

    // Dump the raw row; neither the whole forgotten text nor any distinctive
    // fragment of it may appear.
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
    assert!(!dump.contains(FORGOTTEN));
    assert!(!dump.contains("delayed to Q3"));
    assert!(!dump.contains("private standup"));
}
