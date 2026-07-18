// homn — ingest.rhai (the v2 capture-ingest policy)
// ============================================================================
//
// This is what the gate evaluates for every captured item (screen OCR, audio, dictation,
// email, slack). It decides, before anything touches disk, whether homn may KEEP the item,
// and whether later write-time cloud enrichment is allowed.
//
// Scope variables available in every rule:
//   app           string  — app / window-title / account the capture came from, if known.
//   domain        string  — browser-tab domain (e.g. "github.com"), if known.
//   source_kind   string  — "screen_ocr" | "a11y_tree" | "ambient_audio" | "dictation"
//                          | "email" | "slack" | "github"
//   window_title  string  — full window title, if known.
//   incognito     bool    — true for incognito / private surfaces.
//
// Outcome functions (call exactly one):
//   allow();                deny();          allow_cloud();          redact(kinds);
//   allow_with("id");       deny_with("id"); allow_cloud_with("id"); redact_with(kinds, "id");
//
//   `kinds` is a string ("email_addr,phone") or an array (["email_addr","phone"]) of:
//     api_key token card aadhaar pan person_pii email_addr phone other
//   The always-on scan (api_key/token/card/aadhaar/pan) runs regardless of `redact`.
//
// Helpers: s.matches(glob)  (* and ?),  s.regex("pattern")  (RE2).
//
// Default when no rule fires (FR-026): DENY for incognito + password-manager/vault windows,
// ALLOW otherwise, never AllowCloud. You almost never need to write that default yourself.
//
// Hot-reload (FR-013): editing this file takes effect without restarting capture.

// ============================================================================
// HARD DENIES — never capture, even though the always-on scan would redact secrets anyway.
// ============================================================================

// Password managers and vaults: do not record their windows at all.
if app.matches("KeePass*") { deny_with("vault.keepass"); }
if app.matches("1Password*") { deny_with("vault.1password"); }
if app.matches("Bitwarden*") { deny_with("vault.bitwarden"); }
if window_title.regex("(?i)password") { deny_with("vault.password_window"); }

// Incognito / private browsing: respect the user's privacy intent.
if incognito { deny_with("browser.incognito"); }

// Cloud metadata and secret-browsing on screen.
if domain == "169.254.169.254" { deny_with("ssrf.metadata"); }

// ============================================================================
// REDACT — capture the conversation but strip other people's contact info.
// ============================================================================

// Slack + email + chat: keep the text, redact emails and phone numbers so we don't store
// third-party PII on disk (Invariant 1).
if app.matches("Slack*") || source_kind == "email" {
    redact_with("email_addr,phone", "pii.comms");
}
if app.matches("Zoom*") || source_kind == "ambient_audio" || source_kind == "dictation" {
    redact_with("email_addr,phone", "pii.audio");
}

// ============================================================================
// ALLOW — everything else, subject to the always-on secrets scan.
// ============================================================================

allow_with("default.allow");