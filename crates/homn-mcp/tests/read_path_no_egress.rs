//! Integration test for SC-006 / FR-019: the `recall` and `timeline` MCP tools perform
//! **zero network egress** on the read path (Invariant 2).
//!
//! The behavioural half of this guarantee (the handlers call only the `Brain` trait and every
//! hit carries provenance) is covered by the lib tests `recall_returns_provenance_hits_from_the_brain`,
//! `recall_with_recording_brain_makes_no_other_io`, and `timeline_calls_only_the_brain` — they
//! live in the lib because the `#[tool]` handlers are macro-private.
//!
//! This file holds the **structural** half: the read path is incapable of egress because
//! `homn-mcp` has no direct HTTP-client dependency. A future PR that adds one would have to add
//! a dep, which this test catches.
//!
//! (Task T032 names `tests/read_path_no_egress.rs`; it lives in this crate's `tests/` dir so
//! it can read the crate manifest via `CARGO_MANIFEST_DIR`.)

#![forbid(unsafe_code)]

#[test]
fn read_path_has_no_http_client_dependency() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
    // Inspect only the direct `[dependencies]` block (dev-deps are test-only and excluded).
    let deps_block = text
        .split("[dependencies]")
        .nth(1)
        .and_then(|s| s.split("[dev-dependencies]").next())
        .unwrap_or("");
    for forbidden in ["reqwest", "ureq", "hyper", "isahc", "attohttpc", "curl"] {
        assert!(
            !deps_block.contains(forbidden),
            "homn-mcp must not depend on `{forbidden}` — the read path would gain egress"
        );
    }
}
