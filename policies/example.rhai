// homn — example default.rhai
//
// Copy this to ~/.config/homn/policies/default.rhai and edit to taste.
// Rules are evaluated in deny → ask → allow order; first match wins. Unmatched calls fall
// through to ask. See docs/technical/policy-language.md for the full DSL.
//
// Scope variables available in every rule:
//   tool       string  — e.g. "Bash", "Read", "WebFetch", "mcp__server__tool"
//   cmd        string  — for Bash: the command. Empty otherwise.
//   path       string  — for Read/Edit/Write: the file path. Empty otherwise.
//   url        string  — for WebFetch: the URL. Empty otherwise.
//   cwd        string  — calling session's working directory.
//   home       string  — $HOME.
//   session_id string  — Claude session ULID.
//
// Helpers: `s.matches(pattern)` (shell glob), `s.regex(pattern)` (RE2), plus Rhai's built-in
// string methods (starts_with, ends_with, contains, lower, upper, split, ...).

// ==========================================================================================
// HARD DENIES — these never make it past the policy engine, regardless of any allow below.
// ==========================================================================================

// Destructive Bash outside of scratch dirs
deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny if tool == "Bash" && cmd.regex("\\brm -rf /\\b");
deny if tool == "Bash" && cmd.contains(":(){ :|:& };:");           // fork bomb
deny if tool == "Bash" && cmd.matches("git push --force *");
deny if tool == "Bash" && cmd.matches("git push -f *");

// Cloud metadata service IPs — almost always an SSRF attempt
deny if tool == "WebFetch" && url.contains("169.254.169.254");
deny if tool == "WebFetch" && url.contains("metadata.google.internal");

// Credential files
deny if tool == "Read"  && path.contains("/.ssh/id_");
deny if tool == "Read"  && path.contains("/.aws/credentials");
deny if tool == "Read"  && path.regex("/\\.[a-z]+rc(_history)?$");  // .bashrc / .zshrc history etc.

// ==========================================================================================
// ASKS — production-adjacent, high-stakes. For now homn returns "ask" and Claude will prompt
// you with its own interactive UI. The dedicated TUI prompt lands in T031-T033.
// ==========================================================================================

ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "Bash" && cmd.matches("git push * master");
ask if tool == "Bash" && cmd.matches("git push * release/*");
ask if tool == "Bash" && cmd.contains("sudo ");
ask if tool == "Bash" && cmd.matches("docker push *");
ask if tool == "Bash" && cmd.matches("kubectl apply *");

ask if tool == "WebFetch" && url.contains("internal.");
ask if tool == "WebFetch" && url.regex("^https?://[^/]+\\.prod\\.");

// ==========================================================================================
// ALLOWS — the boring stuff you don't want to babysit.
// ==========================================================================================

// Reads inside your home or repo
allow if tool == "Read" && path.starts_with(home);

// Common dev tooling — Bash
allow if tool == "Bash" && cmd.matches("ls *");
allow if tool == "Bash" && cmd.matches("cat *");
allow if tool == "Bash" && cmd.matches("grep *");
allow if tool == "Bash" && cmd.matches("rg *");
allow if tool == "Bash" && cmd.matches("find * -name *");
allow if tool == "Bash" && cmd.matches("which *");

// Build / test commands
allow if tool == "Bash" && cmd.matches("npm run *");
allow if tool == "Bash" && cmd.matches("npm test *");
allow if tool == "Bash" && cmd.matches("yarn *");
allow if tool == "Bash" && cmd.matches("pnpm *");
allow if tool == "Bash" && cmd.regex("^cargo (build|test|check|clippy|fmt|run|doc)( |$)");
allow if tool == "Bash" && cmd.regex("^(pytest|python -m pytest)( |$)");
allow if tool == "Bash" && cmd.regex("^go (build|test|run|vet|fmt|mod)( |$)");
allow if tool == "Bash" && cmd.regex("^make( |$)");

// Read-only git
allow if tool == "Bash" && cmd.regex("^git (status|log|diff|branch|fetch|remote|show|blame)( |$)");

// Push to feature branches — common safe pattern
allow if tool == "Bash" && cmd.matches("git push origin feat/*");
allow if tool == "Bash" && cmd.matches("git push origin fix/*");
allow if tool == "Bash" && cmd.matches("git push origin docs/*");

// ==========================================================================================
// DEFAULT — anything not matched above gets "ask but learn". Future versions of homn will use
// the audit log to surface promotion suggestions for asks you keep answering the same way.
// ==========================================================================================

ask if true;
