//! The HTTP LLM extraction backend (US5 upgrade over regex) — works for **local Ollama**
//! (`http://localhost:11434/v1/...`, no key, no disclosure) and **any OpenAI-compatible cloud
//! endpoint** (with a key, records `ExtractionSource::Cloud` so the daemon can emit a
//! `DisclosureReceipt` — Invariant 4). Gated behind `extract-llm` so the workspace builds
//! without an HTTP client and the read-path no-egress story stays clean.
//!
//! Uses the OpenAI `/v1/chat/completions` shape, which Ollama also serves, so one code path
//! covers both. The model is asked for strict JSON; parsing is tolerant (extract the first
//! `{...}` object from the response, skip malformed entries) so a small model (qwen2.5:3b)
//! that occasionally emits prose-around-JSON still yields results. **Off the write path,
//! never on read** (FR-020) — the daemon calls this after a `Store::store`.

#![cfg(feature = "extract-llm")]

use std::time::Duration;

use chrono::{DateTime, Utc};
use homn_types::{
    Belief, BeliefId, Commitment, CommitmentId, CommitmentStatus, ExtractionSource, Party,
};
use serde::Deserialize;

use crate::extract::{BeliefExtractor, CommitmentExtractor};

const TIMEOUT: Duration = Duration::from_secs(30);

/// One HTTP LLM extractor — local Ollama or cloud, decided by whether `api_key` is set.
pub struct HttpLlmExtractor {
    /// Base URL, e.g. `http://localhost:11434` (Ollama) or `https://api.anthropic.com` (cloud).
    /// The `/v1/chat/completions` path is appended.
    base_url: String,
    model: String,
    /// `None` → local (no auth, `ExtractionSource::Local`). `Some` → cloud (bearer auth,
    /// `ExtractionSource::Cloud`).
    api_key: Option<String>,
}

impl HttpLlmExtractor {
    /// Local Ollama: `http://localhost:11434`, model `qwen2.5:3b`, no key.
    pub fn local_ollama(model: impl Into<String>) -> Self {
        Self {
            base_url: "http://localhost:11434".to_owned(),
            model: model.into(),
            api_key: None,
        }
    }

    /// Cloud (any OpenAI-compatible endpoint): base URL + model + key.
    pub fn cloud(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key: Some(api_key.into()),
        }
    }

    /// `true` iff this is a cloud call (the daemon emits a `DisclosureReceipt` per call).
    pub fn is_cloud(&self) -> bool {
        self.api_key.is_some()
    }

    fn source(&self) -> ExtractionSource {
        match &self.api_key {
            None => ExtractionSource::Local(self.model.clone()),
            Some(_) => ExtractionSource::Cloud {
                model: self.model.clone(),
            },
        }
    }

    /// POST a chat completion and return the assistant's message content.
    fn chat(&self, system: &str, user: &str) -> anyhow::Result<String> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "stream": false,
            "temperature": 0.0,
        });
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(TIMEOUT))
                .build(),
        );
        let mut req = agent.post(&url);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let mut resp = req.send_json(&body)?;
        let resp: ChatResponse = resp.body_mut().read_json()?;
        Ok(resp.choices[0].message.content.clone())
    }
}

/// OpenAI-style chat response (only the fields we need).
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

impl CommitmentExtractor for HttpLlmExtractor {
    fn extract(&self, text: &str, source_obs: &str, captured_at: DateTime<Utc>) -> Vec<Commitment> {
        let system = "You extract commitments (promises) from text. Reply with ONLY a JSON \
                      object: {\"commitments\":[{\"text\":string,\"owner\":\"me\"|\"<name>\",\
                      \"counterpart\":string|null,\"due\":ISO-8601 date|null}]}. No prose.";
        let user = format!("Text:\n\"\"\"\n{text}\n\"\"\"\nExtract the commitments.");
        let content = match self.chat(system, &user) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "llm commitment extraction call failed");
                return Vec::new();
            }
        };
        parse_commitments(&content, source_obs, captured_at, self.source())
    }
}

impl BeliefExtractor for HttpLlmExtractor {
    fn extract_beliefs(
        &self,
        text: &str,
        source_obs: &str,
        captured_at: DateTime<Utc>,
    ) -> Vec<Belief> {
        let system = "You extract beliefs (positions on a topic) from text. Reply with ONLY a \
                      JSON object: {\"beliefs\":[{\"topic\":string,\"position\":string,\
                      \"confidence\":number}]}. No prose.";
        let user = format!("Text:\n\"\"\"\n{text}\n\"\"\"\nExtract the beliefs.");
        let content = match self.chat(system, &user) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "llm belief extraction call failed");
                return Vec::new();
            }
        };
        parse_beliefs(&content, source_obs, captured_at, self.source())
    }
}

/// Extract the first balanced `{...}` object from `s` (tolerant of prose around the JSON).
fn first_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match (in_str, esc, b) {
            (false, _, b'"') => in_str = true,
            (true, false, b'"') => in_str = false,
            (true, false, b'\\') => esc = true,
            (true, true, _) => esc = false,
            (false, false, b'{') => depth += 1,
            (false, false, b'}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct CommitmentJson {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    counterpart: Option<String>,
    #[serde(default)]
    due: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommitmentsJson {
    #[serde(default)]
    commitments: Vec<serde_json::Value>,
}

fn parse_commitments(
    json: &str,
    source_obs: &str,
    _captured_at: DateTime<Utc>,
    by: ExtractionSource,
) -> Vec<Commitment> {
    let Some(obj) = first_json_object(json) else {
        return Vec::new();
    };
    let parsed: CommitmentsJson = match serde_json::from_str(obj) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    // Parse each entry tolerantly — a malformed entry is skipped, not fatal.
    parsed
        .commitments
        .into_iter()
        .filter_map(|v| serde_json::from_value::<CommitmentJson>(v).ok())
        .filter(|c| c.text.as_deref().is_some_and(|t| !t.trim().is_empty()))
        .map(|c| Commitment {
            id: CommitmentId::new(),
            text: c.text.unwrap().trim().to_owned(),
            owner: party_from(c.owner.as_deref()),
            counterpart: c
                .counterpart
                .filter(|s| !s.is_empty())
                .map(|s| Party::Entity(s.trim().to_owned())),
            due: c.due.as_deref().and_then(parse_due),
            status: CommitmentStatus::Open,
            source_obs: source_obs.to_owned(),
            extracted_by: by.clone(),
            disclosure_receipt: None,
            created_at: Utc::now(),
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct BeliefJson {
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    position: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct BeliefsJson {
    #[serde(default)]
    beliefs: Vec<serde_json::Value>,
}

fn parse_beliefs(
    json: &str,
    source_obs: &str,
    captured_at: DateTime<Utc>,
    by: ExtractionSource,
) -> Vec<Belief> {
    let Some(obj) = first_json_object(json) else {
        return Vec::new();
    };
    let parsed: BeliefsJson = match serde_json::from_str(obj) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    parsed
        .beliefs
        .into_iter()
        .filter_map(|v| serde_json::from_value::<BeliefJson>(v).ok())
        .filter(|b| {
            b.topic.as_deref().is_some_and(|t| !t.trim().is_empty())
                && b.position.as_deref().is_some_and(|p| !p.trim().is_empty())
        })
        .map(|b| Belief {
            id: BeliefId::new(),
            topic: b.topic.unwrap().trim().to_owned(),
            position: b.position.unwrap().trim().to_owned(),
            confidence: b.confidence.unwrap_or(1.0).clamp(0.0, 1.0),
            valid_from: captured_at,
            superseded_at: None,
            source_obs: source_obs.to_owned(),
            extracted_by: by.clone(),
            disclosure_receipt: None,
            created_at: Utc::now(),
        })
        .collect()
}

fn party_from(owner: Option<&str>) -> Party {
    match owner.map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "me" || s.is_empty() => Party::Me,
        Some(name) => Party::Entity(name),
        None => Party::Me,
    }
}

fn parse_due(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    // Try ISO-8601 datetime first, then a bare date.
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(12, 0, 0))
                .map(|dt| dt.and_utc())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commitments_json_with_prose_around_it() {
        let resp = "Sure! Here are the commitments:\n{\"commitments\":[{\"text\":\"send the pricing quote\",\"owner\":\"me\",\"counterpart\":\"Marco\",\"due\":\"2026-07-17\"}]}\nHope that helps!";
        let cs = parse_commitments(
            resp,
            "obs-1",
            Utc::now(),
            ExtractionSource::Local("qwen2.5:3b".into()),
        );
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "send the pricing quote");
        assert!(matches!(cs[0].owner, Party::Me));
        assert!(matches!(&cs[0].counterpart, Some(Party::Entity(n)) if n == "Marco"));
        assert!(cs[0].due.is_some());
        assert!(matches!(cs[0].extracted_by, ExtractionSource::Local(_)));
    }

    #[test]
    fn parses_beliefs_json() {
        let resp = "{\"beliefs\":[{\"topic\":\"the brain\",\"position\":\"agidb is good enough\",\"confidence\":0.8}]}";
        let bs = parse_beliefs(resp, "obs-2", Utc::now(), ExtractionSource::Regex);
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].topic, "the brain");
        assert!((bs[0].confidence - 0.8).abs() < 1e-6);
    }

    #[test]
    fn skips_malformed_entries() {
        let resp =
            "{\"commitments\":[{\"text\":\"good one\",\"owner\":\"me\"},{\"owner\":\"me\"}]}";
        let cs = parse_commitments(resp, "obs", Utc::now(), ExtractionSource::Regex);
        assert_eq!(cs.len(), 1, "the empty-text entry is dropped");
    }

    #[test]
    fn returns_empty_on_unparseable() {
        assert!(
            parse_commitments("no json here", "obs", Utc::now(), ExtractionSource::Regex)
                .is_empty()
        );
    }

    /// Hit the real local Ollama if running — skipped in CI. Run with:
    /// `cargo test -p homnd --features extract-llm -- --ignored llm_local_ollama`
    #[tokio::test]
    #[ignore = "needs a local Ollama serving qwen2.5:3b at localhost:11434"]
    async fn llm_local_ollama_extracts_a_promise() {
        // Ollama is sync (ureq); run on a blocking thread.
        let ext = HttpLlmExtractor::local_ollama("qwen2.5:3b");
        let cs = tokio::task::spawn_blocking(move || {
            ext.extract(
                "I'll send the pricing quote to Marco by Friday.",
                "obs-live",
                Utc::now(),
            )
        })
        .await
        .unwrap();
        assert!(
            !cs.is_empty(),
            "the model extracted at least one commitment"
        );
        assert!(cs.iter().any(|c| c.text.to_lowercase().contains("pricing")
            || c.text.to_lowercase().contains("quote")));
    }
}
