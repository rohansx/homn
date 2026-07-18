//! The redaction bank — the second stage of the gate pipeline (US3 / T036).
//!
//! Given pre-redaction text and the set of kinds the ingest policy asked to strip, this module
//! runs a registry of regex detectors, finds all sensitive spans, resolves overlaps, and
//! rewrites the text with `[REDACTED:<kind>]` placeholders. It returns the redacted text plus
//! plaintext-free [`RedactionSpan`]s (locators + kind + detector id) which the pipeline turns
//! into [`RedactionRef`]s once the audit ledger assigns each a `ledger_seq`.
//!
//! A fixed set of **always-on** detectors (API keys, bearer tokens, cards, Aadhaar, PAN) runs
//! unconditionally — the contract calls this the "always-on secrets scan", so a policy that says
//! `allow` still cannot persist a credit-card number. Policy-requested kinds (email, phone,
//! person-PII, other) layer on top. See [`specs/002-ambient-memory/contracts/gate-pipeline.md`]
//! stage 2 and FR-012.
//!
//! `PersonPii` is detector-less by default: detecting it well needs an NER model or a dictionary,
//! and v1 ships neither. The kind is still carried through the type system so a policy can request
//! it and a future detector can fill the gap without a schema change.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use homn_types::{RedactionKind, SpanRef};
use regex::Regex;

/// One detector's finding, before it has been assigned a ledger position.
///
/// Carries everything except `ledger_seq` (which the audit ledger assigns when the pipeline
/// writes the receipt). The span locates the *placeholder* in the redacted text, never the
/// original bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionSpan {
    /// What kind of sensitive content was removed.
    pub kind: RedactionKind,
    /// Locator of the placeholder in the redacted text (offset, length). Not the original content.
    pub span: SpanRef,
    /// Identifier of the detector that produced this span (e.g. `"bank.card"`).
    pub policy_id: String,
}

/// The result of a [`RedactionBank::redact`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    /// The rewritten text, with placeholders in place of every detected span.
    pub text: String,
    /// One [`RedactionSpan`] per replacement, in text order.
    pub spans: Vec<RedactionSpan>,
}

/// One detector: a compiled regex, the kind it tags, and an optional Luhn check.
struct Detector {
    policy_id: &'static str,
    kind: RedactionKind,
    re: Regex,
    /// If set, a candidate match is only kept if it passes a Luhn checksum (cards only).
    luhn: bool,
}

impl Detector {
    const fn new(policy_id: &'static str, kind: RedactionKind, re: Regex) -> Self {
        Self {
            policy_id,
            kind,
            re,
            luhn: false,
        }
    }
}

/// The registry of redaction detectors, run by the gate's redact stage.
#[derive(Clone)]
pub struct RedactionBank {
    detectors: Arc<[Detector]>,
}

impl Default for RedactionBank {
    fn default() -> Self {
        // The always-on secrets scan: high-entropy credentials and payment/auth secrets.
        // These run for every item regardless of policy, so an `allow` cannot persist a card.
        let always_on: [Detector; 5] = [
            // Bearer tokens: "Bearer eyJ..." / "Authorization: Bearer ..."
            Detector::new(
                "bank.token",
                RedactionKind::Token,
                Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._\-+=/]{16,}").unwrap(),
            ),
            // AWS access-key ids and OpenAI-style "sk-" keys, plus `<name>_key=<long>` assignments.
            Detector::new(
                "bank.api_key",
                RedactionKind::ApiKey,
                Regex::new(
                    r#"(?:\bAKIA[0-9A-Z]{16}\b|\bsk-[A-Za-z0-9]{20,}\b|(?i)\b(?:api[_-]?key|secret|token|passwd|password)\s*[:=]\s*['\"]?[A-Za-z0-9_\-/+=]{20,}['\"]?)"#,
                ).unwrap(),
            ),
            // Payment card numbers (13–19 digits, separators only *between* digits), Luhn-validated.
            Detector {
                policy_id: "bank.card",
                kind: RedactionKind::Card,
                re: Regex::new(r"\b\d(?:[ -]?\d){12,18}\b").unwrap(),
                luhn: true,
            },
            // Indian Aadhaar: 12 digits in 4-4-4 groups, first 1–9.
            Detector::new(
                "bank.aadhaar",
                RedactionKind::Aadhaar,
                Regex::new(r"\b[1-9]\d{3}[\s-]?\d{4}[\s-]?\d{4}\b").unwrap(),
            ),
            // Indian PAN: 5 letters + 4 digits + 1 letter.
            Detector::new(
                "bank.pan",
                RedactionKind::Pan,
                Regex::new(r"\b[A-Z]{5}[0-9]{4}[A-Z]\b").unwrap(),
            ),
        ];

        // Policy-opt-in detectors: email and phone. PersonPii/Other carry no detector yet.
        let opt_in: [Detector; 2] = [
            Detector::new(
                "bank.email",
                RedactionKind::EmailAddr,
                Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap(),
            ),
            Detector::new(
                "bank.phone",
                RedactionKind::Phone,
                // +country optional, 10–15 digits, allowing spaces/dots/dashes between groups.
                Regex::new(r"(?:\+?\d[\s.\-]?){10,15}").unwrap(),
            ),
        ];

        let mut all: Vec<Detector> = Vec::with_capacity(always_on.len() + opt_in.len());
        all.extend(always_on);
        // Opt-in detectors are only active when their kind is requested by the policy.
        all.extend(opt_in);
        Self {
            detectors: all.into(),
        }
    }
}

impl RedactionBank {
    /// Redact `text`, running the always-on detectors plus any opt-in detectors whose kind is in
    /// `requested`. Returns the rewritten text and the spans (in order).
    ///
    /// Overlaps are resolved leftmost-wins: once a span is taken, any detector match that starts
    /// inside it is dropped. This keeps the placeholder set unambiguous and the offsets stable.
    pub fn redact(&self, text: &str, requested: &[RedactionKind]) -> Redacted {
        let mut hits: Vec<(usize, usize, RedactionKind, &'static str)> = Vec::new();

        for d in self.detectors.iter() {
            // Always-on kinds: ApiKey, Token, Card, Aadhaar, Pan. Opt-in: EmailAddr, Phone.
            let is_always_on = matches!(
                d.kind,
                RedactionKind::ApiKey
                    | RedactionKind::Token
                    | RedactionKind::Card
                    | RedactionKind::Aadhaar
                    | RedactionKind::Pan
            );
            let active = is_always_on || requested.contains(&d.kind);
            if !active {
                continue;
            }
            for m in d.re.find_iter(text) {
                let candidate = (m.start(), m.end(), d.kind, d.policy_id);
                if d.luhn && !luhn_valid(&text[m.start()..m.end()]) {
                    continue;
                }
                hits.push(candidate);
            }
        }

        // Sort by start, then by longest-first so a longer secret wins a tie at the same start.
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));

        // Resolve overlaps: drop any hit whose start falls within an already-kept span.
        let mut kept: Vec<(usize, usize, RedactionKind, &'static str)> = Vec::new();
        let mut covered_end: usize = 0;
        for h in hits {
            if h.0 < covered_end {
                continue; // starts inside a prior kept span
            }
            kept.push(h);
            covered_end = h.1;
        }

        // Rebuild the string with placeholders, recording each placeholder's (offset, len).
        let mut out = String::with_capacity(text.len());
        let mut spans = Vec::with_capacity(kept.len());
        let mut cursor = 0usize;
        for (s, e, kind, policy_id) in kept {
            out.push_str(&text[cursor..s]);
            let placeholder = placeholder(kind);
            let offset = out.len() as u32;
            out.push_str(&placeholder);
            let len = placeholder.len() as u32;
            spans.push(RedactionSpan {
                kind,
                span: SpanRef { offset, len },
                policy_id: policy_id.to_owned(),
            });
            cursor = e;
        }
        out.push_str(&text[cursor..]);

        Redacted { text: out, spans }
    }
}

/// The `[REDACTED:<kind>]` placeholder for a kind, using the serde snake_case name.
fn placeholder(kind: RedactionKind) -> String {
    // serde rename_all = snake_case; mirror it here so the on-disk text matches the wire type.
    let name = match kind {
        RedactionKind::ApiKey => "api_key",
        RedactionKind::Token => "token",
        RedactionKind::Card => "card",
        RedactionKind::Aadhaar => "aadhaar",
        RedactionKind::Pan => "pan",
        RedactionKind::PersonPii => "person_pii",
        RedactionKind::EmailAddr => "email_addr",
        RedactionKind::Phone => "phone",
        RedactionKind::Other => "other",
    };
    format!("[REDACTED:{name}]")
}

/// Luhn checksum (mod-10) over the digits in `s`. Cards only — false on non-digit noise.
fn luhn_valid(s: &str) -> bool {
    let digits: Vec<u8> = s
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for &d in digits.iter().rev() {
        let mut x = d as u32;
        if double {
            x *= 2;
            if x > 9 {
                x -= 9;
            }
        }
        sum += x;
        double = !double;
    }
    sum % 10 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(k: &[RedactionKind]) -> Vec<RedactionKind> {
        k.to_vec()
    }

    #[test]
    fn always_on_redacts_credit_card_even_when_not_requested() {
        let bank = RedactionBank::default();
        // A valid test Visa PAN (4242 ... 4242), Luhn-valid.
        let r = bank.redact("card 4242 4242 4242 4242 expires 12/30", &[]);
        assert_eq!(r.text, "card [REDACTED:card] expires 12/30");
        assert_eq!(r.spans.len(), 1);
        assert_eq!(r.spans[0].kind, RedactionKind::Card);
        assert_eq!(r.spans[0].policy_id, "bank.card");
        // The span points at the placeholder, not the digits.
        assert_eq!(r.spans[0].span.offset, 5);
        assert_eq!(r.spans[0].span.len as usize, "[REDACTED:card]".len());
    }

    #[test]
    fn luhn_rejects_non_card_digit_runs() {
        // 13 random digits that fail Luhn must NOT be redacted as a card.
        let bank = RedactionBank::default();
        let r = bank.redact("order 1234567890123 done", &[]);
        assert_eq!(r.text, "order 1234567890123 done", "non-Luhn run kept");
        assert!(r.spans.is_empty());
    }

    #[test]
    fn api_key_assignment_is_redacted_always() {
        let bank = RedactionBank::default();
        let r = bank.redact("api_key = AKIAIOSFODNN7EXAMPLE used", &[]);
        assert!(r.text.contains("[REDACTED:api_key]"));
        assert!(!r.text.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn bearer_token_is_redacted_always() {
        let bank = RedactionBank::default();
        let r = bank.redact("Authorization: Bearer abcdefghij1234567890plus", &[]);
        assert!(r.text.contains("[REDACTED:token]"));
    }

    #[test]
    fn aadhaar_and_pan_are_redacted_always() {
        let bank = RedactionBank::default();
        let r = bank.redact("pan ABCDE1234F aadhaar 1234 5678 9012", &[]);
        assert!(r.text.contains("[REDACTED:pan]"));
        assert!(r.text.contains("[REDACTED:aadhaar]"));
    }

    #[test]
    fn email_is_only_redacted_when_requested() {
        let bank = RedactionBank::default();
        let by_default = bank.redact("ping chris@acme.com please", &[]);
        assert_eq!(by_default.text, "ping chris@acme.com please");

        let with_email = bank.redact("ping chris@acme.com please", &kinds(&[RedactionKind::EmailAddr]));
        assert_eq!(with_email.text, "ping [REDACTED:email_addr] please");
    }

    #[test]
    fn phone_is_only_redacted_when_requested() {
        let bank = RedactionBank::default();
        let r = bank.redact("call +1 415 555 2671 now", &kinds(&[RedactionKind::Phone]));
        assert!(r.text.contains("[REDACTED:phone]"));
        let def = bank.redact("call +1 415 555 2671 now", &[]);
        assert!(!def.text.contains("[REDACTED:phone]"));
    }

    #[test]
    fn overlapping_spans_resolve_leftmost_wins() {
        // A long api_key assignment whose value is a 40-digit run that a 16-digit card detector
        // would also match inside. The api_key match starts earlier (at `secret`) and must win;
        // the card sub-match starts inside it and is dropped.
        let bank = RedactionBank::default();
        let r = bank.redact("secret = 4242424242424242424242424242424242424242 key", &[]);
        assert_eq!(r.spans.len(), 1, "one span, not two overlapping");
        assert_eq!(r.spans[0].kind, RedactionKind::ApiKey);
    }

    #[test]
    fn multiple_distinct_spans_are_all_redacted_in_order() {
        let bank = RedactionBank::default();
        let r = bank.redact(
            "key sk-abc123def4567890ghi12 and card 4242 4242 4242 4242",
            &[],
        );
        assert_eq!(r.spans.len(), 2);
        assert!(r.spans[0].span.offset < r.spans[1].span.offset);
        assert!(r.text.contains("[REDACTED:api_key]"));
        assert!(r.text.contains("[REDACTED:card]"));
    }

    #[test]
    fn clean_text_is_unchanged() {
        let bank = RedactionBank::default();
        let r = bank.redact("just a normal status update about lunch", &[]);
        assert_eq!(r.text, "just a normal status update about lunch");
        assert!(r.spans.is_empty());
    }
}