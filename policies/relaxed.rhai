// homn — relaxed.rhai (trusted-project profile)
// ============================================================================
//
// A permissive profile for a project you trust: the agent gets the full dev loop without
// prompts, and homn only steps in for the genuinely dangerous or irreversible.
//
// Use it as your default:  cp policies/relaxed.rhai ~/.config/homn/policies/default.rhai
//
// See policies/default.rhai for the scope-variable and helper reference.

// ---- HARD DENIES — the irreversible ones still stand -----------------------
deny if tool == "Bash" && cmd.regex("\\brm -rf /\\b");
deny if tool == "Bash" && cmd.regex("\\brm -rf ~\\s*$");
deny if tool == "Bash" && cmd.regex("\\b(mkfs\\.|dd\\b.*of=/dev/)");
deny if tool == "Bash" && cmd.contains(":(){ :|:& };:");
deny if tool == "Bash" && cmd.regex("\\bgit push\\b.*(--force|-f)\\b.*\\b(main|master)\\b");

deny if tool == "WebFetch" && url.contains("169.254.169.254");
deny if tool == "Read" && path.contains("/.ssh/id_");
deny if tool == "Read" && path.contains("/.aws/credentials");

// ---- ASKS — only the truly high-stakes -------------------------------------
ask if tool == "Bash" && cmd.matches("git push * main");
ask if tool == "Bash" && cmd.matches("git push * master");
ask if tool == "Bash" && cmd.regex("^(npm|cargo) publish( |$)");
ask if tool == "Bash" && cmd.matches("terraform apply *");
ask if tool == "Bash" && cmd.matches("kubectl apply *");

// ---- ALLOWS — the whole dev loop -------------------------------------------
allow if tool == "Read";
allow if tool == "Edit" && path.starts_with(home);
allow if tool == "Write" && path.starts_with(home);
allow if tool == "Bash" && cmd.regex("^(npm|pnpm|yarn|cargo|go|make|pytest|python)( |$)");
// All git except push — `git push` is handled by the deny/ask rules above.
allow if tool == "Bash" && cmd.starts_with("git ") && !cmd.contains("git push");
allow if tool == "Bash" && cmd.matches("git push origin feat/*");
allow if tool == "Bash" && cmd.matches("git push origin fix/*");
allow if tool == "Bash" && cmd.matches("docker *");

// ---- EVERYTHING ELSE — allow, since this is a trusted project --------------
allow if true;
