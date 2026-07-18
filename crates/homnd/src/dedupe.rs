//! The dedupe stage — pipeline stage 3 (T028).
//!
//! Near-duplicate collapse using the post-redaction [`Observation::content_hash`]. Because the
//! hash is over redacted text, two captures that differ only in a secret (both redacted to the
//! same placeholder) collapse — the desired "at-least-once upstream, exactly-once stored"
//! behavior that makes crash-replay cheap (R7). v1 keeps a bounded LRU of recent hashes; the
//! brain's own dedupe is authoritative and layers on top.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::VecDeque;

use homn_types::Observation;

/// A bounded set of recently-seen content hashes, for near-duplicate collapse.
///
/// Bounded so a long-running daemon doesn't grow unbounded: old hashes age out, accepting a
/// small re-store rate for ancient content over an unbounded memory footprint. The bound is
/// generous enough that intra-session OCR repeats (the common case) are always caught.
pub struct Dedupe {
    capacity: usize,
    seen: VecDeque<u64>,
}

impl Dedupe {
    /// Build a dedupe map with the given capacity (recent-hash window).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            seen: VecDeque::with_capacity(capacity.max(1)),
        }
    }
}

/// The default window: 8192 recent hashes (~one screen-capture session's worth).
impl Default for Dedupe {
    fn default() -> Self {
        Self::new(8192)
    }
}

impl Dedupe {
    /// Returns `true` if `obs` is a near-duplicate of something seen recently (and records it
    /// either way). The caller drops the item on `true` — the gate already produced the receipt.
    pub fn is_duplicate(&mut self, obs: &Observation) -> bool {
        let h = obs.content_hash;
        let dup = self.seen.iter().any(|x| *x == h);
        if self.seen.len() >= self.capacity {
            self.seen.pop_front();
        }
        self.seen.push_back(h);
        dup
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use homn_types::{Provenance, SourceKind};

    fn obs(hash: u64) -> Observation {
        Observation {
            id: ulid::Ulid::new(),
            source: SourceKind::ScreenOcr,
            app: Some("Code".to_owned()),
            captured_at: Utc::now(),
            ingested_at: Utc::now(),
            text: "x".to_owned(),
            redactions: vec![],
            session: None,
            speaker: None,
            content_hash: hash,
            provenance: Provenance {
                source_id: "x".to_owned(),
                upstream_ref: "r".to_owned(),
            },
        }
    }

    #[test]
    fn first_sight_is_not_a_duplicate() {
        let mut d = Dedupe::new(64);
        assert!(!d.is_duplicate(&obs(1)));
    }

    #[test]
    fn second_sight_of_same_hash_is_a_duplicate() {
        let mut d = Dedupe::new(64);
        assert!(!d.is_duplicate(&obs(7)));
        assert!(d.is_duplicate(&obs(7)));
    }

    #[test]
    fn old_hashes_age_out_so_ancient_content_can_re_store() {
        let mut d = Dedupe::new(2);
        d.is_duplicate(&obs(1));
        d.is_duplicate(&obs(2));
        d.is_duplicate(&obs(3)); // evicts hash 1
        assert!(
            !d.is_duplicate(&obs(1)),
            "hash 1 aged out → not a duplicate anymore"
        );
    }
}
