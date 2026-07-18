//! The gate pipeline — the single place a [`RawCapture`] becomes a storable
//! [`Observation`] (US3 / T034–T042).
//!
//! Stages, in order, all inside [`Gate::run`]:
//!
//! 1. **POLICY** ([`IngestPolicy`]) → [`PolicyDecision`] (`Deny | Redact(kinds) | Allow | AllowCloud`)
//! 2. **REDACT** ([`RedactionBank`]) → redacted text + [`RedactionSpan`]s
//! 3. **OBSERVE** → construct the [`Observation`] (content-hash, provenance, ids) from the
//!    *redacted* text only
//!
//! Hard rules from [`contracts/gate-pipeline.md`] enforced here:
//!
//! - **R-1 gate precedes store** — `Gate::run` is the *only* producer of an `Observation`;
//!   [`GateOutput::Stored`] is built entirely from post-redaction text. There is no other
//!   constructor in this crate.
//! - **R-2 fail closed** — any error in POLICY or REDACT, or a policy `Deny`, yields
//!   [`GateOutput::Dropped`] with a reason; nothing is persisted.
//! - **R-4 ledger completeness** — every decision produces a [`DecisionReceipt`] which the caller
//!   writes to the audit ledger. Redactions are surfaced as [`RedactionRef`]s whose
//!   `ledger_seq` the caller fills from the receipt's ledger position.
//!
//! Receipt writing and watermark advance live in the caller (`homnd`), not here: the gate is pure
//! and synchronous, which makes it trivially testable and keeps the ledger concerns in one place.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use chrono::Utc;
use homn_types::{
    IngestOutcome, Observation, Provenance, RawCapture, Receipt, RedactionRef,
};
use ulid::Ulid;

use crate::policy::{IngestContext, IngestPolicy, PolicyDecision};
use crate::redaction::{Redacted, RedactionBank, RedactionSpan};
use crate::IngestAction;

/// What the gate did with one captured item.
#[derive(Debug, Clone)]
pub enum GateOutput {
    /// The item passed the gate and may be persisted. The [`Observation`] is built entirely from
    /// redacted text; `redactions` are plaintext-free refs awaiting `ledger_seq` from the caller.
    Stored {
        /// The storable observation (post-redaction only).
        observation: Observation,
        /// Redaction refs, one per redacted span; `ledger_seq` is 0 until the caller back-fills it
        /// from the audit ledger.
        redactions: Vec<RedactionRef>,
        /// Whether the policy authorized later write-time cloud enrichment.
        permits_cloud: bool,
    },
    /// The item was dropped (deny or gate error). Nothing is persistable.
    Dropped {
        /// Why it was dropped — used to build the decision receipt.
        outcome: IngestOutcome,
        /// The rule that fired, if any.
        rule_id: Option<String>,
    },
}

/// The configured gate: a policy + a redaction bank. Cloneable; cheap to share across tasks.
#[derive(Clone)]
pub struct Gate {
    policy: IngestPolicy,
    bank: RedactionBank,
}

impl Gate {
    /// Build a gate from a compiled policy and the default redaction bank.
    pub fn new(policy: IngestPolicy) -> Self {
        Self {
            policy,
            bank: RedactionBank::default(),
        }
    }

    /// Build a gate with an explicit redaction bank (tests / custom detectors).
    pub fn with_bank(policy: IngestPolicy, bank: RedactionBank) -> Self {
        Self { policy, bank }
    }

    /// Run a single captured item through the gate. Pure: writes no disk, touches no ledger.
    pub fn run(&self, capture: &RawCapture) -> GateOutput {
        // 1. POLICY
        let decision = self.policy.evaluate(&ingest_ctx(capture));
        match decision.action {
            IngestAction::Deny => GateOutput::Dropped {
                outcome: IngestOutcome::Deny,
                rule_id: decision.rule_id,
            },
            IngestAction::Allow | IngestAction::AllowCloud | IngestAction::Redact(_) => {
                self.pass_redact(capture, decision)
            }
        }
    }
}

impl Gate {
    /// Stages 2–3: redact (only the kinds the policy named, on top of the always-on scan) and
    /// build the observation from the redacted text.
    fn pass_redact(&self, capture: &RawCapture, decision: PolicyDecision) -> GateOutput {
        let requested = match &decision.action {
            IngestAction::Redact(kinds) => kinds.clone(),
            _ => Vec::new(),
        };
        let Redacted { text, spans } = self.bank.redact(&capture.text, &requested);

        // 3. OBSERVE — construct the Observation purely from `text` (post-redaction).
        let content_hash = Observation::compute_content_hash(
            capture.source,
            capture.app.as_deref(),
            &text,
        );
        let observation = Observation {
            id: Ulid::new(),
            source: capture.source,
            app: capture.app.clone(),
            captured_at: capture.captured_at,
            ingested_at: Utc::now(),
            text,
            redactions: redaction_refs(&spans),
            session: None, // sessionizer runs after the gate, in homnd
            speaker: capture.speaker,
            content_hash,
            provenance: Provenance {
                source_id: source_id_for(capture),
                upstream_ref: capture.upstream_ref.clone(),
            },
        };

        GateOutput::Stored {
            observation,
            redactions: redaction_refs(&spans),
            permits_cloud: decision.action.permits_cloud(),
        }
    }
}

/// The ingest context for a raw capture, mapping source fields onto the policy scope.
fn ingest_ctx(c: &RawCapture) -> IngestContext {
    IngestContext {
        app: c.app.clone().unwrap_or_default(),
        domain: String::new(), // populated by the caller from screenpipe's UI metadata, if any
        source_kind: source_kind_str(c.source),
        window_title: String::new(),
        incognito: false,
    }
}

/// `SourceKind` → its serde snake_case name, for the policy scope.
fn source_kind_str(s: homn_types::SourceKind) -> String {
    use homn_types::SourceKind as S;
    match s {
        S::ScreenOcr => "screen_ocr",
        S::A11yTree => "a11y_tree",
        S::AmbientAudio => "ambient_audio",
        S::Dictation => "dictation",
        S::Email => "email",
        S::Slack => "slack",
        S::GitHub => "github",
    }
    .to_owned()
}

/// A stable source_id for a capture, used in provenance and to key the watermark.
fn source_id_for(c: &RawCapture) -> String {
    source_kind_str(c.source)
}

/// Convert detector spans into plaintext-free refs (ledger_seq left 0 for the caller to fill).
fn redaction_refs(spans: &[RedactionSpan]) -> Vec<RedactionRef> {
    spans
        .iter()
        .map(|s| RedactionRef {
            kind: s.kind,
            span: s.span,
            policy_id: s.policy_id.clone(),
            ledger_seq: 0,
        })
        .collect()
}

/// Build the decision receipt for a gate outcome, for the audit ledger (R-4).
pub fn decision_receipt(out: &GateOutput) -> Receipt {
    use homn_types::{DecisionReceipt, Receipt as R};
    match out {
        GateOutput::Stored {
            observation,
            permits_cloud: _,
            ..
        } => R::Decision(DecisionReceipt {
            outcome: IngestOutcome::Allow,
            policy_id: None, // filled by the caller from the policy's rule_id, when applicable
            observation_ref: Some(observation.id.to_string()),
            at: observation.ingested_at,
        }),
        GateOutput::Dropped { outcome, rule_id } => R::Decision(DecisionReceipt {
            outcome: *outcome,
            policy_id: rule_id.clone(),
            observation_ref: None,
            at: Utc::now(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::IngestContext;
    use homn_types::{RawCapture, RedactionKind, SourceKind};

    fn capture(text: &str) -> RawCapture {
        RawCapture {
            upstream_ref: "row-1".to_owned(),
            source: SourceKind::ScreenOcr,
            app: Some("Firefox".to_owned()),
            captured_at: Utc::now(),
            text: text.to_owned(),
            speaker: None,
        }
    }

    fn allow_all_policy() -> IngestPolicy {
        IngestPolicy::compile("allow();").unwrap()
    }

    #[test]
    fn clean_capture_is_stored_with_no_redactions() {
        let gate = Gate::new(allow_all_policy());
        let out = gate.run(&capture("the meeting went fine"));
        match out {
            GateOutput::Stored {
                observation,
                redactions,
                ..
            } => {
                assert_eq!(observation.text, "the meeting went fine");
                assert!(redactions.is_empty());
                assert_eq!(observation.source, SourceKind::ScreenOcr);
                assert_eq!(observation.provenance.upstream_ref, "row-1");
            }
            _ => panic!("expected Stored"),
        }
    }

    #[test]
    fn credit_card_in_a_clean_capture_is_still_redacted_r1_and_always_on_scan() {
        // R-1 + the always-on secrets scan: even an `allow` policy must not persist a PAN.
        let gate = Gate::new(allow_all_policy());
        let out = gate.run(&capture("card 4242 4242 4242 4242 ok"));
        match out {
            GateOutput::Stored {
                observation,
                redactions,
                ..
            } => {
                assert_eq!(observation.text, "card [REDACTED:card] ok");
                assert_eq!(redactions.len(), 1);
                assert_eq!(redactions[0].kind, RedactionKind::Card);
                assert_eq!(redactions[0].ledger_seq, 0, "caller fills ledger_seq");
                assert!(observation.text.contains("[REDACTED:card]"));
                assert!(!observation.text.contains("4242"));
            }
            _ => panic!("expected Stored"),
        }
    }

    #[test]
    fn deny_policy_drops_the_item_and_records_no_observation() {
        let gate = Gate::new(IngestPolicy::compile("deny();").unwrap());
        let out = gate.run(&capture("anything"));
        match out {
            GateOutput::Dropped {
                outcome,
                rule_id,
            } => {
                assert_eq!(outcome, IngestOutcome::Deny);
                assert_eq!(rule_id.as_deref(), None);
            }
            _ => panic!("expected Dropped"),
        }
    }

    #[test]
    fn redact_policy_strips_requested_kinds_in_addition_to_always_on() {
        let gate = Gate::new(IngestPolicy::compile(r#"redact("email_addr");"#).unwrap());
        let out = gate.run(&capture("ping chris@acme.com card 4242 4242 4242 4242"));
        match out {
            GateOutput::Stored {
                observation,
                redactions,
                ..
            } => {
                assert!(observation.text.contains("[REDACTED:email_addr]"));
                assert!(observation.text.contains("[REDACTED:card]"));
                assert!(!observation.text.contains("chris@acme.com"));
                assert!(!observation.text.contains("4242"));
                // Two distinct kinds, both plaintext-free.
                assert_eq!(redactions.len(), 2);
            }
            _ => panic!("expected Stored"),
        }
    }

    #[test]
    fn allow_cloud_propagates_the_cloud_flag() {
        let gate = Gate::new(IngestPolicy::compile("allow_cloud();").unwrap());
        let out = gate.run(&capture("a transcript"));
        match out {
            GateOutput::Stored { permits_cloud, .. } => assert!(permits_cloud),
            _ => panic!("expected Stored"),
        }
    }

    #[test]
    fn content_hash_distinguishes_redacted_text_from_original() {
        // The dedupe key is over the redacted text, so two captures that differ only in a secret
        // (both redacted to the same placeholder) collapse to the same hash — the desired dedupe.
        let gate = Gate::new(allow_all_policy());
        let a = gate.run(&capture("card 4242 4242 4242 4242 note"));
        let b = gate.run(&capture("card 4111 1111 1111 1111 note")); // different PAN, same placeholder
        let (ha, hb) = match (a, b) {
            (GateOutput::Stored { observation: a, .. }, GateOutput::Stored { observation: b, .. }) => {
                (a.content_hash, b.content_hash)
            }
            _ => panic!("expected both Stored"),
        };
        assert_eq!(
            ha, hb,
            "two captures redacting to the same placeholder must hash identically (dedupe)"
        );
    }

    #[test]
    fn runtime_policy_error_fails_closed_as_dropped_deny() {
        let gate = Gate::new(IngestPolicy::compile("nope_undefined();").unwrap());
        let out = gate.run(&capture("anything"));
        // A runtime error fails closed: the policy evaluate() returns Deny, so the gate drops.
        match out {
            GateOutput::Dropped { outcome, .. } => assert_eq!(outcome, IngestOutcome::Deny),
            _ => panic!("runtime error must fail closed, not store"),
        }
    }

    #[test]
    fn decision_receipt_for_deny_names_the_outcome() {
        let gate = Gate::new(IngestPolicy::compile("deny();").unwrap());
        let out = gate.run(&capture("x"));
        let r = decision_receipt(&out);
        match r {
            Receipt::Decision(d) => {
                assert_eq!(d.outcome, IngestOutcome::Deny);
                assert!(d.observation_ref.is_none());
            }
            _ => panic!("expected a decision receipt"),
        }
    }

    #[test]
    fn decision_receipt_for_stored_names_the_observation_ref() {
        let gate = Gate::new(allow_all_policy());
        let out = gate.run(&capture("clean"));
        let r = decision_receipt(&out);
        match r {
            Receipt::Decision(d) => {
                assert_eq!(d.outcome, IngestOutcome::Allow);
                assert!(d.observation_ref.is_some());
            }
            _ => panic!("expected a decision receipt"),
        }
    }
}