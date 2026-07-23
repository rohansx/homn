//! US4 integration test (T045/T048): `forget` removes memories from recall and writes a
//! tamper-evident `DeletionReceipt` to the audit ledger. Feature-gated behind `brain-agidb`
//! (needs the agidb store + unlearn).
//!
//! Invariant 3 / FR-023/024: after forget, the matched memory stops surfacing in recall, and a
//! `DeletionReceipt { scope, match_count, at }` is chained in the ledger proving the scope
//! **without re-exposing the forgotten content**.

#![cfg(feature = "brain-agidb")]

use std::sync::Arc;

use homn_audit::Db;
use homn_types::{ForgetScope, Receipt};
use homnd::store::{AgidbStore, Store};

fn brain_dir(label: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "homnd-forget-{}-{label}-{}.agidb",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}

async fn make_brain(dir: &std::path::Path) -> Arc<agidb::Agidb> {
    let b = agidb::Agidb::open_with(
        agidb::AgidbConfig::new(dir).with_extractor(agidb::ExtractorSetup::Null),
    )
    .await
    .unwrap();
    for text in [
        "Sarah promised the quote by Friday",
        "Priya owes the API spec by Tuesday",
        "standup notes from Monday morning",
    ] {
        b.observe_with(text, "test").await.unwrap();
    }
    b.flush().await.unwrap();
    Arc::new(b)
}

#[tokio::test]
async fn forget_removes_from_recall_and_writes_a_deletion_receipt() {
    let dir = brain_dir("forget");
    let brain = make_brain(&dir).await;

    // Before forget: "Sarah" recall surfaces the promise.
    let before = brain.recall_cue("Sarah promised").await.unwrap();
    assert!(
        before
            .matches
            .iter()
            .any(|m| m.text.contains("quote by Friday")),
        "recall surfaces the Sarah episode before forget: {:?}",
        before.matches
    );

    // Forget via the store (Pattern scope → recall resolves the episode → unlearn).
    let store = AgidbStore::new(brain.clone());
    let receipt = store
        .forget(&ForgetScope::Pattern("Sarah promised".to_owned()))
        .await
        .unwrap();
    assert!(
        receipt.match_count >= 1,
        "at least the Sarah episode was removed (match_count={})",
        receipt.match_count
    );
    assert!(matches!(receipt.scope, ForgetScope::Pattern(_)));

    // After forget: "Sarah" recall no longer surfaces the promise.
    let after = brain.recall_cue("Sarah promised").await.unwrap();
    assert!(
        !after
            .matches
            .iter()
            .any(|m| m.text.contains("quote by Friday")),
        "the Sarah episode is gone from recall after forget: {:?}",
        after.matches
    );

    // The other episodes are untouched.
    let priya = brain.recall_cue("Priya API spec").await.unwrap();
    assert!(
        priya.matches.iter().any(|m| m.text.contains("API spec")),
        "forget was scoped — Priya episode still recallable"
    );

    // A DeletionReceipt is chained in the audit ledger (Invariant 3).
    let audit_dir = brain_dir("audit");
    std::fs::create_dir_all(&audit_dir).unwrap();
    let audit = Db::open(audit_dir.join("audit.db")).await.unwrap();
    let entry = audit
        .append_receipt(&Receipt::Deletion(receipt.clone()))
        .await
        .unwrap();
    assert!(entry.seq >= 1, "the receipt got a ledger seq (receipt_id)");
    let verify = audit.verify_ledger().await.unwrap();
    assert!(
        verify.first_bad_seq.is_none(),
        "ledger still verifies after the append"
    );

    drop(store);
    drop(brain);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&audit_dir);
}

#[tokio::test]
async fn forget_time_range_targets_only_the_window() {
    use chrono::TimeZone;
    let dir = brain_dir("timerange");
    let brain = make_brain(&dir).await; // all three observed at ~now

    // A window covering only the far future → nothing to forget.
    let future = chrono::Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
    let future2 = chrono::Utc.with_ymd_and_hms(2099, 12, 31, 0, 0, 0).unwrap();
    let store = AgidbStore::new(brain.clone());
    let receipt = store
        .forget(&ForgetScope::TimeRange {
            from: future,
            to: future2,
        })
        .await
        .unwrap();
    assert_eq!(
        receipt.match_count, 0,
        "no episodes in the far-future window"
    );

    // A wide window covering now → forgets the observed episodes.
    let past = chrono::Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let receipt = store
        .forget(&ForgetScope::TimeRange {
            from: past,
            to: future2,
        })
        .await
        .unwrap();
    assert!(
        receipt.match_count >= 1,
        "the wide window forgets the observed episodes (match_count={})",
        receipt.match_count
    );

    drop(store);
    drop(brain);
    let _ = std::fs::remove_dir_all(&dir);
}
