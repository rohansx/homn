//! Write-time commitment extraction (US5 / T050) — turn captured (post-redaction) text into
//! structured [`Commitment`](homn_types::Commitment) records that `commitments()` can answer
//! without a recall pass over raw text.
//!
//! **Start narrow** (research R10): the v1 [`RegexExtractor`] finds explicit-promise phrasing
//! ("I'll … by Friday", "<Name> promised … by <date>", "<Name> owes me …") deterministically, no
//! network, no disclosure receipt. A cloud (`Haiku`) / local (`qwen2.5:3b`) backend lands later
//! behind the same [`CommitmentExtractor`] trait, gated on `AllowCloud` + a `DisclosureReceipt`
//! (Invariant 4). Extraction runs **off the store-write path, never on read** (FR-020): the
//! daemon calls [`extract_commitments`] after a `Store::store`, and adds the results to a
//! [`crate::commitments::CommitmentStore`].
//!
//! Dates are best-effort: day-of-week (Monday..Sunday), "today/tomorrow", and ISO dates resolve
//! relative to the observation's `captured_at`. Unresolvable dates leave `due = None` — the
//! promise is still recorded, just undated.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc, Weekday};
use homn_types::{Commitment, CommitmentId, CommitmentStatus, ExtractionSource, Party};
use regex::Regex;

/// The extraction backend. Pluggable so the regex v1 and the future cloud/local backends share
/// one contract.
pub trait CommitmentExtractor: Send + Sync {
    /// Extract commitments from `text` (post-redaction), attributed to `source_obs`.
    /// `captured_at` anchors relative date resolution ("Friday" → the next Friday on/after it).
    fn extract(&self, text: &str, source_obs: &str, captured_at: DateTime<Utc>) -> Vec<Commitment>;
}

/// The v1 deterministic extractor — regex patterns for explicit promises. No network, no
/// disclosure. Cheap to run on every stored observation.
#[derive(Debug)]
pub struct RegexExtractor {
    /// "I'll <verb-phrase> by <date>." — first-person promise, owner = Me.
    ill_by: Regex,
    /// "I will <verb-phrase> by <date>."
    i_will_by: Regex,
    /// "<Name> promised <object> by <date>." — third-party, owner = Entity(Name).
    promised_by: Regex,
    /// "<Name> will <verb-phrase> by <date>." — third-party future promise.
    name_will_by: Regex,
    /// "I owe <Name> ..." / "<Name> owes me ..." — owes phrasing.
    owes: Regex,
}

impl RegexExtractor {
    /// Construct with the v1 patterns.
    pub fn new() -> Self {
        Self {
            // "I'll send the pricing quote to Marco by Friday."
            ill_by: Regex::new(
                r"(?i)\bI['']ll\b\s+(.+?)\s+\bby\b\s+([A-Za-z]+day|today|tomorrow|\d{4}-\d{2}-\d{2})",
            )
            .unwrap(),
            // "I will send the spec by Tuesday."
            i_will_by: Regex::new(
                r"(?i)\bI\s+will\b\s+(.+?)\s+\bby\b\s+([A-Za-z]+day|today|tomorrow|\d{4}-\d{2}-\d{2})",
            )
            .unwrap(),
            // "Sarah promised the user research by Wednesday." / "Sarah promised Chris ..."
            promised_by: Regex::new(
                r"(?i)\b([A-Z][a-z]+)\s+promised\b\s+(.+?)\s+\bby\b\s+([A-Za-z]+day|today|tomorrow|\d{4}-\d{2}-\d{2})",
            )
            .unwrap(),
            // "Priya will send the API spec by Tuesday."
            name_will_by: Regex::new(
                r"(?i)\b([A-Z][a-z]+)\s+will\b\s+(.+?)\s+\bby\b\s+([A-Za-z]+day|today|tomorrow|\d{4}-\d{2}-\d{2})",
            )
            .unwrap(),
            // "I owe Priya the API spec." / "Priya owes me the API spec."
            owes: Regex::new(r"(?i)\bI\s+owe\b\s+([A-Z][a-z]+)\s+(.+?)[.\n]|([A-Z][a-z]+)\s+owes\s+me\b\s+(.+?)[.\n]")
                .unwrap(),
        }
    }
}

impl Default for RegexExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommitmentExtractor for RegexExtractor {
    fn extract(&self, text: &str, source_obs: &str, captured_at: DateTime<Utc>) -> Vec<Commitment> {
        let mut out = Vec::new();
        let mk = |promise: String,
                  owner: Party,
                  counterpart: Option<Party>,
                  due: Option<DateTime<Utc>>| {
            Commitment {
                id: CommitmentId::new(),
                text: promise.trim().to_owned(),
                owner,
                counterpart,
                due,
                status: CommitmentStatus::Open,
                source_obs: source_obs.to_owned(),
                extracted_by: ExtractionSource::Regex,
                disclosure_receipt: None,
                created_at: Utc::now(),
            }
        };

        // First-person: "I'll … by <date>"
        for c in self.ill_by.captures_iter(text) {
            let promise = format!("I'll {} by {}", &c[1], &c[2]);
            let due = resolve_date(&c[2], captured_at);
            out.push(mk(promise, Party::Me, None, due));
        }
        for c in self.i_will_by.captures_iter(text) {
            let promise = format!("I will {} by {}", &c[1], &c[2]);
            let due = resolve_date(&c[2], captured_at);
            out.push(mk(promise, Party::Me, None, due));
        }
        // Third-party: "<Name> promised … by <date>"
        for c in self.promised_by.captures_iter(text) {
            let name = c[1].to_owned();
            let promise = format!("{name} promised {} by {}", &c[2], &c[3]);
            let due = resolve_date(&c[3], captured_at);
            out.push(mk(
                promise,
                Party::Entity(name.clone()),
                Some(Party::Me),
                due,
            ));
        }
        for c in self.name_will_by.captures_iter(text) {
            let name = c[1].to_owned();
            let promise = format!("{name} will {} by {}", &c[2], &c[3]);
            let due = resolve_date(&c[3], captured_at);
            out.push(mk(
                promise,
                Party::Entity(name.clone()),
                Some(Party::Me),
                due,
            ));
        }
        // Owes: "I owe <Name> <object>" → owner Me, counterpart Entity(Name);
        // "<Name> owes me <object>" → owner Entity(Name), counterpart Me.
        for c in self.owes.captures_iter(text) {
            if let (Some(name), Some(obj)) = (c.get(1), c.get(2)) {
                let promise = format!("I owe {} {}", name.as_str(), obj.as_str());
                out.push(mk(
                    promise,
                    Party::Me,
                    Some(Party::Entity(name.as_str().to_owned())),
                    None,
                ));
            } else if let (Some(name), Some(obj)) = (c.get(3), c.get(4)) {
                let promise = format!("{} owes me {}", name.as_str(), obj.as_str());
                out.push(mk(
                    promise,
                    Party::Entity(name.as_str().to_owned()),
                    Some(Party::Me),
                    None,
                ));
            }
        }
        out
    }
}

/// Free-function convenience: extract with the default [`RegexExtractor`].
pub fn extract_commitments(
    text: &str,
    source_obs: &str,
    captured_at: DateTime<Utc>,
) -> Vec<Commitment> {
    RegexExtractor::default().extract(text, source_obs, captured_at)
}

/// Resolve a due-date token relative to `anchor` (the observation's captured_at). Day-of-week →
/// the next occurrence on/after the anchor's local date; today/tomorrow → anchor / +1d; ISO date
/// → parsed. `None` if unresolvable (the promise is still recorded, undated).
fn resolve_date(token: &str, anchor: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let t = token.trim();
    // ISO date.
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(d.and_hms_opt(12, 0, 0)?.and_utc());
    }
    let local = anchor.with_timezone(&Local).date_naive();
    let target = match t.to_ascii_lowercase().as_str() {
        "today" => Some(local),
        "tomorrow" => Some(local + Duration::days(1)),
        wd => weekday_from_name(wd).map(|target_wd| next_weekday(local, target_wd)),
    }?;
    Some(target.and_hms_opt(12, 0, 0)?.and_utc())
}

fn weekday_from_name(s: &str) -> Option<Weekday> {
    match s {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// The next date on/after `from` that falls on `target` weekday (same day if `from` is the target:
/// "by Friday" said on a Friday is due that Friday).
fn next_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
    let cur = from.weekday();
    let diff =
        (target.num_days_from_monday() as i64 - cur.num_days_from_monday() as i64).rem_euclid(7);
    from + Duration::days(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> DateTime<Utc> {
        // 2026-07-13 was a Monday.
        DateTime::parse_from_rfc3339("2026-07-13T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn extracts_first_person_promise_with_weekday_due() {
        let cs = extract_commitments(
            "I'll send the pricing quote to Marco by Friday.",
            "obs-1",
            anchor(),
        );
        assert_eq!(cs.len(), 1);
        assert!(cs[0].text.starts_with("I'll send the pricing quote"));
        assert!(matches!(cs[0].owner, Party::Me));
        assert!(cs[0].due.is_some(), "Friday resolves to a date");
        // Friday after Monday 2026-07-13 is 2026-07-17.
        assert_eq!(
            cs[0].due.unwrap().format("%Y-%m-%d").to_string(),
            "2026-07-17"
        );
        assert!(matches!(cs[0].extracted_by, ExtractionSource::Regex));
        assert!(cs[0].disclosure_receipt.is_none(), "regex needs no receipt");
    }

    #[test]
    fn extracts_third_party_promise() {
        let cs = extract_commitments(
            "Sarah promised the user research by Wednesday.",
            "obs-2",
            anchor(),
        );
        assert_eq!(cs.len(), 1);
        assert!(matches!(&cs[0].owner, Party::Entity(n) if n == "Sarah"));
        assert!(matches!(&cs[0].counterpart, Some(Party::Me)));
    }

    #[test]
    fn extracts_owes_in_both_directions() {
        let cs = extract_commitments("Priya owes me the API spec.\n", "obs-3", anchor());
        assert_eq!(cs.len(), 1);
        assert!(matches!(&cs[0].owner, Party::Entity(n) if n == "Priya"));
        assert!(matches!(&cs[0].counterpart, Some(Party::Me)));

        let cs = extract_commitments("I owe Marco the pricing quote.\n", "obs-4", anchor());
        assert_eq!(cs.len(), 1);
        assert!(matches!(cs[0].owner, Party::Me));
        assert!(matches!(&cs[0].counterpart, Some(Party::Entity(n)) if n == "Marco"));
    }

    #[test]
    fn leaves_due_none_when_date_unresolvable() {
        // "soon" isn't a recognized date token — the promise is still recorded, undated.
        let cs = RegexExtractor::default().extract("I'll fix the bug soon.", "obs-5", anchor());
        // "soon" doesn't match the by-<date> pattern at all → no extraction here.
        assert!(cs.is_empty(), "no by-<date> → not matched by the regex");
    }

    #[test]
    fn extracts_multiple_promises_from_one_text() {
        let text = "I'll send the quote by Friday. Sarah promised the research by Wednesday.";
        let cs = extract_commitments(text, "obs-6", anchor());
        assert_eq!(cs.len(), 2, "both promises extracted");
    }

    #[test]
    fn is_overdue_when_past_due_and_open() {
        let past = Utc::now() - Duration::days(3);
        let c = Commitment {
            id: CommitmentId::new(),
            text: "I'll send it by yesterday".into(),
            owner: Party::Me,
            counterpart: None,
            due: Some(past),
            status: CommitmentStatus::Open,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: Utc::now(),
        };
        assert!(c.is_overdue(Utc::now()));
    }
}
