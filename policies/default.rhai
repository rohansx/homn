// homn — default.rhai (the recommended starting policy)
// ============================================================================
//
// This is what `homn rule edit` seeds and what `homn install` points at. Copy it to
// ~/.config/homn/policies/default.rhai and edit to taste — the daemon hot-reloads on save.
//
// Rules are evaluated in DENY -> ASK -> ALLOW order; the first match wins. Anything that
// matches no rule falls through to the catch-all `ask if true` at the bottom, so homn is
// conservative by default: an unknown call is surfaced to you, never silently allowed.
//
// Scope variables available in every rule:
//   tool        string  — "Bash", "Read", "Edit", "Write", "WebFetch", "mcp__server__tool"
//   cmd         string  — for Bash: the command. Empty otherwise.
//   path        string  — for Read/Edit/Write: the file path. Empty otherwise.
//   url         string  — for WebFetch: the URL. Empty otherwise.
//   cwd         string  — calling session's working directory.
//   home        string  — $HOME.
//   session_id  string  — Claude session ULID.
//
// Helpers: `s.matches(pattern)` (shell glob: * and ?), `s.regex(pattern)` (RE2), plus Rhai's
// built-in string methods (starts_with, ends_with, contains, lower, upper, split, ...).
//
// See docs/technical/policy-language.md for the full DSL reference.

// ============================================================================
// HARD DENIES — never allowed, regardless of any allow rule below.
// ============================================================================

// Destructive filesystem operations
deny if tool == "Bash" && cmd.contains("rm -rf") && !cwd.starts_with("/tmp");
deny if tool == "Bash" && cmd.regex("\\brm -rf /\\b");
deny if tool == "Bash" && cmd.regex("\\brm -rf ~\\s*$");
deny if tool == "Bash" && cmd.contains(":(){ :|:& };:");            // fork bomb
deny if tool == "Bash" && cmd.regex("\\bmkfs\\.");                  // formatting a filesystem
deny if tool == "Bash" && cmd.regex("\\bdd\\b.*\\bof=/dev/");       // raw write to a device

// History rewriting / force pushes — almost never what you want unattended
deny if tool == "Bash" && cmd.matches("git push --force *");
deny if tool == "Bash" && cmd.matches("git push -f *");
deny if tool == "Bash" && cmd.contains("git reset --hard");

// Cloud metadata service IPs — almost always an SSRF attempt
deny if tool == "WebFetch" && url.contains("169.254.169.254");
deny if tool == "WebFetch" && url.contains("metadata.google.internal");

// Credential + secret files — don't let an agent read these
deny if tool == "Read" && path.contains("/.ssh/id_");
deny if tool == "Read" && path.contains("/.aws/credentials");
deny if tool == "Read" && path.contains("/.config/gh/hosts.yml");
deny if tool == "Read" && path.ends_with(".pem");
deny if tool == "Read" && path.ends_with(".env");

// Editing dotfile rc scripts is a common persistence vector
deny if tool == "Write" && path.regex("/\\.[a-z]+rc$");
deny if tool == "Edit"  && path.regex("/\\.[a-z]+rc$");

// ============================================================================
// ASKS — production-adjacent or high-stakes. homn surfaces these to you (TUI prompt).
// ============================================================================

// Pushing to a protected branch
ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "Bash" && cmd.matches("git push * master");
ask if tool == "Bash" && cmd.matches("git push * release/*");

// Privilege escalation + infrastructure mutation
ask if tool == "Bash" && cmd.contains("sudo ");
ask if tool == "Bash" && cmd.matches("docker push *");
ask if tool == "Bash" && cmd.matches("kubectl apply *");
ask if tool == "Bash" && cmd.matches("terraform apply *");
ask if tool == "Bash" && cmd.regex("^npm publish( |$)");
ask if tool == "Bash" && cmd.regex("^cargo publish( |$)");

// Package installs — usually fine, but worth a glance
ask if tool == "Bash" && cmd.regex("^(npm|pnpm|yarn) (add|install)( |$)");
ask if tool == "Bash" && cmd.regex("^pip install( |$)");
ask if tool == "Bash" && cmd.regex("^cargo add( |$)");

// Network fetches to internal / production hosts
ask if tool == "WebFetch" && url.contains("internal.");
ask if tool == "WebFetch" && url.regex("^https?://[^/]+\\.prod\\.");

// ============================================================================
// ALLOWS — the boring, safe stuff you don't want to babysit.
// ============================================================================

// Reads + edits inside your home directory
allow if tool == "Read" && path.starts_with(home);
allow if tool == "Edit" && path.starts_with(home);
allow if tool == "Write" && path.starts_with(home);

// Read-only shell inspection
allow if tool == "Bash" && cmd.matches("ls *");
allow if tool == "Bash" && cmd.matches("cat *");
allow if tool == "Bash" && cmd.matches("grep *");
allow if tool == "Bash" && cmd.matches("rg *");
allow if tool == "Bash" && cmd.matches("find * -name *");
allow if tool == "Bash" && cmd.matches("which *");
allow if tool == "Bash" && cmd.regex("^(pwd|whoami|date|env|echo)( |$)");

// Build / test / lint — the inner dev loop
allow if tool == "Bash" && cmd.matches("npm run *");
allow if tool == "Bash" && cmd.matches("npm test *");
allow if tool == "Bash" && cmd.matches("pnpm run *");
allow if tool == "Bash" && cmd.regex("^cargo (build|test|check|clippy|fmt|run|doc|bench)( |$)");
allow if tool == "Bash" && cmd.regex("^(pytest|python -m pytest)( |$)");
allow if tool == "Bash" && cmd.regex("^go (build|test|run|vet|fmt|mod)( |$)");
allow if tool == "Bash" && cmd.regex("^make( |$)");

// Read-only git
allow if tool == "Bash" && cmd.regex("^git (status|log|diff|branch|fetch|remote|show|blame)( |$)");

// Pushing to feature branches — the common safe pattern
allow if tool == "Bash" && cmd.matches("git push origin feat/*");
allow if tool == "Bash" && cmd.matches("git push origin fix/*");
allow if tool == "Bash" && cmd.matches("git push origin docs/*");

// ============================================================================
// DEFAULT — anything that matches no rule above is surfaced to you automatically:
// the engine's implicit fallthrough is `ask`. As you answer those unmatched calls
// the same way, `homn learning` offers to promote them into allow/deny rules.
//
// Do NOT add an explicit `ask if true` here. Rules run deny -> ask -> allow, so a
// catch-all `ask` would fire before every `allow` rule above and shadow them all —
// nothing would ever be allowed. The implicit default already does the right thing.
// ============================================================================
