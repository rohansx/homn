# ADR-0007 — Integrate GitHub Spec Kit alongside docs/

**Status**: Accepted

## Context

After the initial design pass (`docs/product/overview.md`, `docs/architecture/`, `docs/phases/`, `docs/research/`, etc.), we adopted [GitHub Spec Kit](https://github.com/github/spec-kit) for the spec-driven development workflow on Claude Code. Spec Kit installs:

- A set of slash commands under `.claude/skills/` (`/speckit-constitution`, `/speckit-specify`, `/speckit-plan`, `/speckit-tasks`, `/speckit-implement`, etc.)
- A `.specify/` directory with templates, memory (constitution), workflows.
- A pattern: each feature gets `specs/<###-feature-slug>/` containing `spec.md`, `plan.md`, `research.md`, `data-model.md`, `quickstart.md`, `contracts/`, `tasks.md`.

This conflicts (in form, not content) with our existing `docs/phases/` structure: spec-kit thinks in *features*, our docs think in *phases*.

## Decision

Both coexist. Each has a defined role:

- **`docs/` is the long-form reference.** Architecture, ADRs, deep-dive technical specs, research notes, risk register, go-to-market. Reads like a book.
- **`specs/<###-feature>/` is the working spec for an in-progress feature.** Reads like a sprint plan. Generated/edited via spec-kit slash commands.
- **`.specify/memory/constitution.md`** is the **single source of truth** for project principles. `docs/` references it; ADRs cite specific principles when justifying a deviation.

Phase 1 maps to `specs/001-policy-engine/` (one feature with 5 user stories P1–P5). Each subsequent phase becomes a new spec-kit feature directory.

### Rejected alternatives

| Alternative                                            | Reason rejected                                                                |
|--------------------------------------------------------|--------------------------------------------------------------------------------|
| Use spec-kit only; delete `docs/`                      | Spec-kit `spec.md` is feature-shaped; doesn't fit long-form architecture or ADRs.|
| Use `docs/` only; skip spec-kit                        | Loses the Claude Code slash command workflow + the gate-check discipline.       |
| Symlink `specs/<n>/plan.md` → `docs/phases/phase-N.md` | Spec-kit expects standalone files with constitution-check tables; phase docs are narrative. |
| Duplicate content fully                                 | Drift guaranteed within weeks.                                                  |

## How they interconnect

- **`specs/.../plan.md`** has thin "Phase 0 — Research" and "Phase 1 — Design" sections that **link out** to `docs/research/`, `docs/architecture/`, and ADRs. No duplication.
- **ADRs live in `docs/architecture/adr/`** because they outlive any single spec-kit feature.
- **Constitution is in `.specify/memory/constitution.md`** because spec-kit's workflows look for it there; `docs/` references it via relative link.
- **`CLAUDE.md`** (project root) is the AI-agent entry point. Points at the active spec-kit feature and the constitution.

## Consequences

- New phases initialize a new spec-kit feature directory (`specs/002-face/`, `specs/003-brain/`).
- The `docs/phases/*.md` files become **narrative companion docs** for the more rigid spec-kit files — useful for the human reader and for marketing/launch context.
- Slash commands `/speckit-clarify`, `/speckit-checklist`, `/speckit-analyze` are available for spec quality work; we use them between phases.
- The repo has two top-level directories of design content (`docs/` and `specs/`). This is intentional and documented in the README + CLAUDE.md.
- Spec-kit's `/speckit-implement` command can be invoked from a Claude session inside this directory to execute `tasks.md` items one at a time. Mattpocock's `tdd` and `to-issues` skills (already installed globally) compose with it.
