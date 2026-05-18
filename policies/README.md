# Sample policies

homn evaluates Rhai rule files in **deny → ask → allow** order; the first match wins.
Anything that matches no rule falls through to `ask`, so homn is conservative by default.

These files are **samples** — copy one to `~/.config/homn/policies/default.rhai` and edit it.
The daemon hot-reloads on save (a syntactically broken edit keeps the previous ruleset live).

| File | Profile | Use it when |
|------|---------|-------------|
| [`default.rhai`](./default.rhai) | **Balanced** — denies the destructive, asks about the high-stakes, allows the dev loop. | The recommended starting point for most people. |
| [`strict.rhai`](./strict.rhai) | **Locked down** — allows only read-only operations, asks about everything else. | Unattended / overnight agent runs, or repos where you want to review every mutation. |
| [`relaxed.rhai`](./relaxed.rhai) | **Trusted** — full dev loop with no prompts; only the irreversible is denied. | A personal project you trust the agent in. |
| [`project-example.rhai`](./project-example.rhai) | **Project overlay** — stacks on top of `default.rhai` for one repo. | You want to tighten or loosen policy for a single repository. |

## Quick start

```sh
mkdir -p ~/.config/homn/policies
cp policies/default.rhai ~/.config/homn/policies/default.rhai
# or: homn rule edit   (seeds default.rhai and opens $EDITOR)
```

## Project-scoped policies

homn loads `default.rhai` first, then a file named after the repo slug (e.g. `cloakpipe.rhai`)
if one exists in the policies directory. Project rules stack on top — within each verb group
the project file is evaluated first — so a project file tightens or loosens one repo without
touching your global default.

## Inspecting a policy

```sh
homn rule list                          # every rule, grouped by verb
homn rule trace Bash "rm -rf /"          # which rules match a given call, and why
homn rule trace Read "$HOME/notes.md"
```

The full DSL — scope variables, helpers, evaluation semantics — is documented in
[`docs/technical/policy-language.md`](../docs/technical/policy-language.md).
