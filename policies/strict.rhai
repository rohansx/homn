// homn — strict.rhai (locked-down profile)
// ============================================================================
//
// A conservative profile: deny everything destructive, allow only genuinely read-only
// operations, and ask about everything else. Good for unattended / overnight agent runs,
// or for a project where you want to review every mutation.
//
// Use it as your default:  cp policies/strict.rhai ~/.config/homn/policies/default.rhai
//
// See policies/default.rhai for the scope-variable and helper reference.

// ---- HARD DENIES -----------------------------------------------------------
deny if tool == "Bash" && cmd.contains("rm -rf");
deny if tool == "Bash" && cmd.regex("\\b(mkfs\\.|dd\\b.*of=/dev/)");
deny if tool == "Bash" && cmd.contains(":(){ :|:& };:");
deny if tool == "Bash" && cmd.regex("\\bgit push\\b.*(--force|-f)\\b");
deny if tool == "Bash" && cmd.contains("git reset --hard");
deny if tool == "Bash" && cmd.contains("sudo ");
deny if tool == "Bash" && cmd.contains("curl ") && cmd.contains("| sh");
deny if tool == "Bash" && cmd.contains("curl ") && cmd.contains("| bash");

deny if tool == "WebFetch" && url.contains("169.254.169.254");
deny if tool == "WebFetch" && url.contains("metadata.google.internal");

deny if tool == "Read" && path.contains("/.ssh/");
deny if tool == "Read" && path.contains("/.aws/");
deny if tool == "Read" && path.ends_with(".pem");
deny if tool == "Read" && path.ends_with(".env");

// ---- ALLOWS — read-only only -----------------------------------------------
allow if tool == "Read" && path.starts_with(home) && !path.contains("/.");
allow if tool == "Bash" && cmd.regex("^(ls|cat|pwd|whoami|date|echo|which)( |$)");
allow if tool == "Bash" && cmd.regex("^(grep|rg)( |$)");
allow if tool == "Bash" && cmd.regex("^git (status|log|diff|branch|show|blame)( |$)");

// ---- EVERYTHING ELSE -------------------------------------------------------
// No catch-all rule needed: anything that matches none of the allows above falls
// through to `ask` implicitly. An explicit `ask if true` would run before the
// allow rules (deny -> ask -> allow) and shadow them — don't add one.
