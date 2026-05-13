# ADR-0001 — Name the project `homn`

**Status**: Accepted

## Context

The original product overview used the placeholder `brm` (vaguely "brain" / "bridge"). Open options considered: `brm`, `homd` (homunculus daemon), `prsd` (presence), `gatd` (gate), `veild`, `brd`.

Constraints:
- Short (≤5 chars), lowercase, daemon-style — fits the author's aesthetic alongside `clipd`, `primd`, `wardn`, `workz`, `cloakpipe`.
- Easy to type as a binary and pronounce verbally.
- No existing crates.io conflict.
- Has a thematic reading that justifies it without being on the nose.

## Decision

Project name is `homn`. Binary is `homn`. Daemon process is invoked as `homn daemon`. Subcommands match: `homn rule`, `homn log`, `homn face`, `homn run claude`, `homn hook <event>`.

**Reading**: short for *homunculus* — a small alchemical figure that lives at the edge of things and watches what happens. It's not an acronym; it's a vibe that fits the product (the daemon is a small watcher peripheral to your real work).

**Tagline**: *"the homunculus for your coding agents."*

## Consequences

- crates.io name available as of 2026-05.
- Pronunciation: "hom-n" or just "home". Both work.
- The "homunculus" reading is **optional**. We never put a literal alchemical figure in the face. The reading is for vibes; the product is for productivity.
- If we ever expand the brand (a SaaS team-sync plan, a hosted variant), `homn.dev` is the domain target.
