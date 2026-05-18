// homn — project-scoped policy example
// ============================================================================
//
// homn loads `default.rhai` first, then a project file named after the repo slug
// (e.g. `cloakpipe.rhai`) if one exists. Project rules STACK ON TOP of the default —
// they're evaluated first within each verb group, so a project file can tighten or
// loosen policy for one repo without touching your global default.
//
// To use this for a repo:
//   cp policies/project-example.rhai ~/.config/homn/policies/<your-repo-slug>.rhai
//
// This example assumes a repo where `main` is protected and deploys are sensitive.

// Tighten: in THIS repo, never touch the deploy script unattended.
deny if tool == "Edit"  && path.ends_with("deploy.sh");
deny if tool == "Write" && path.ends_with("deploy.sh");
deny if tool == "Bash"  && cmd.contains("./deploy.sh");

// Tighten: any database migration command gets surfaced.
ask if tool == "Bash" && cmd.contains("migrate");
ask if tool == "Bash" && cmd.contains("prisma db push");

// Loosen: this repo's test runner is safe to run freely, even though it shells out.
allow if tool == "Bash" && cmd.starts_with("./scripts/test");
