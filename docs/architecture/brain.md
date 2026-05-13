# Architecture — Layer 3: The Brain (ctxgraph integration)

> A personal RAG system that lives in a daemon and actually knows what you're working on. The layer that turns `homn` from a tool into a coherent product.

## What it does

`ctxgraph` (existing local-first context graph engine — separate repo) wires into `homn` as a **memory subsystem**. The daemon ingests events from across your dev environment; `ctxgraph` stores them as a bi-temporal knowledge graph; the face surfaces what matters when it matters.

Layer 3 doesn't change *what `homn` does*. It changes *what `homn` knows*, which makes:

- **Policy sharper** — rules can reference state (`ask unless ctxgraph.has_open_pr_passing_ci(...)`).
- **The face context-aware** — thought bubbles surface real-world open loops, not invented ones.
- **The audit log meaningful** — *"this was denied because the last 3 sessions in this repo failed CI"*.

## Why this layer ships last

Two reasons:

1. **Policy + face are useful without it.** Phase 1 and Phase 2 produce a working product. Phase 3 is the multiplier.
2. **ctxgraph readiness is unresolved.** Before Phase 3 begins, we audit ctxgraph's current API surface vs. what layer 3 needs (schema extensions, ingestor pattern, Rhai-callable helpers). This is called out as a major risk in [risks/known-unknowns.md](../risks/known-unknowns.md).

## Inputs (event sources)

Each is opt-in. `homn` ships with **conservative defaults**: git on, transcripts OFF by default (privacy reasons — see below). Everything else off until configured.

| Source              | Mechanism                                  | Default | What it captures                              |
|---------------------|--------------------------------------------|---------|-----------------------------------------------|
| Git activity        | `fsnotify` on watched dirs                 | ON      | commits, branches, PRs, push events           |
| Shell history       | zsh/bash hook calling daemon socket        | OFF     | commands run, exit codes, cwd                 |
| Claude transcripts  | `SessionStart` + `Stop` hooks              | **OFF** | tool calls, decisions, conversation summaries |
| Build/test output   | wrapped commands or filewatch on build dirs| OFF     | pass/fail, error counts, durations            |
| Calendar (opt)      | ical read-only sync                        | OFF     | upcoming meetings, focus blocks               |
| Gmail (opt)         | scoped to drafts + sent labels             | OFF     | open replies, unsent drafts                   |
| Browser tabs (opt)  | Chrome extension                           | OFF     | docs you're reading, GH PRs open              |

### Privacy posture — important deviation from the pasted overview

The pasted overview says *"brm ships with sane defaults (git + claude transcripts on; everything else off)"*. **We're changing transcript ingestion to OFF by default.**

Reason: Claude transcripts contain pasted secrets, env vars, API keys, source code, business data, customer names. Default-on ingestion makes `~/.local/share/ctxgraph/store.db` the most sensitive file on the machine.

Mitigation when the user opts in:

- **Regex redaction layer.** Strip common secret patterns (`sk-...`, `AKIA...`, `Bearer ...`, anything matching the user's configured allowlist of env-var names) before persistence.
- **Per-project allow/deny.** A `~/.config/homn/ctxgraph.toml` lists projects ingestion is allowed for; new projects default to deny.
- **Encryption at rest.** SQLite encryption with a key derived from the user's keychain (macOS Keychain / Linux Secret Service).

This costs a little Phase 3 velocity but removes the privacy footgun.

## What ctxgraph stores

`ctxgraph`'s existing model (entities + relations + bi-temporal timestamps + provenance) already fits. Node types we add:

- **projects** (repos)
- **sessions** (Claude Code conversations)
- **files** (paths edited / read)
- **commands** (things run in shells)
- **commits / PRs**
- **people** (mentioned in commits, mail, calendar)
- **open loops** (drafted but unsent mail, unanswered messages, paused tasks)

Relations: `session edited file`, `command failed in cwd`, `PR assigned to person at time`, `session decided X via rule Y`.

**Bi-temporal** means tracking both *when something happened* and *when we learned about it*. This matters because *"what was I working on last Tuesday"* and *"what did I think I was working on last Tuesday"* are different queries — and the second one is often more useful.

## What layer 3 enables

### Session resumption

Starting `claude` in `~/dev/cloakpipe` triggers a thought bubble on the face (or a TUI line on entry, if face is off):

> *"Last time in cloakpipe (3 days ago): you were drafting the Venice integration email but didn't send it. Open recap?"*

Click → injects a context summary into the new session via the `UserPromptSubmit` hook. Claude starts already oriented.

Success metric: do users click "open recap" >30% of the time? If yes, the context is good. If no, recalibrate the summarization prompt.

### Open-loop surfacing

`ctxgraph` tracks things you started but didn't finish:

- Gmail drafts not sent in 48h
- PRs you opened but didn't merge
- Branches with commits ahead but no PR
- Conversations that mentioned a TODO but no follow-up

The face periodically surfaces one open loop at a time as a thought bubble.

**Cadence is the whole game**: max once per hour, and only when you're idle (≥N minutes of no daemon activity). Anything more frequent crosses into nag territory and people mute the face within a week.

Success metric: nudge dismissal rate <40%. Above that means we're being annoying.

### Context-aware policy rules

Rhai rules can query ctxgraph state:

```rhai
// ask for `git push main` unless there's an open PR with passing CI for this branch
ask if tool == "Bash"
   && cmd.matches("git push * main")
   && !ctxgraph.has_open_pr_passing_ci(cwd, branch);

// auto-allow read of files you've edited in the last 24h
allow if tool == "Read"
   && ctxgraph.recently_edited(path, hours: 24);

// deny WebFetch to a domain mentioned in a "do not contact" wiki page
deny if tool == "WebFetch"
   && ctxgraph.matches_wiki_tag(url.domain, "do-not-contact");
```

This is where the layers genuinely fuse: **policy gets sharper because it has memory**. Every existing tool has either rules or memory; nothing has rules *over* memory.

### Searchable memory

Hover the face, type a query, get an answer with provenance:

```
> when did i last talk to naman about prior inventions

march 12 at 14:23 — claude session "utkrushta-onboarding"
  context: discussed verbal agreement that OSS projects remain yours
  follow-up: send email confirming list (still in drafts, not sent)
```

This is what makes the face's hover panel useful beyond "what's happening right now". Your second brain, in a daemon, with a face. `ctxgraph` finally has a consumer surface.

## What ctxgraph already has vs. what's new for layer 3

`ctxgraph` already has (no change required):

- The graph engine
- Entity resolution (GLiNER)
- Bi-temporal storage
- SQLite FTS5 indexing
- An MCP server (we expose its queries through `homn`'s MCP server)

What's new in `homn`'s layer 3 work:

- **Ingestors** — the source connectors (git watcher, shell hook, transcript subscriber, optional cal/gmail/browser).
- **Schema extensions** — adding project/session/command/loop nodes without breaking existing ctxgraph users. Versioned. See [ADR-0005](adr/0005-ctxgraph-separate.md).
- **Surfacing logic** — deciding *when* to show a nudge, *which* one, *how often*. This is harder than it sounds and is what makes or breaks the layer.
- **Consumer API** — Rhai helpers like `ctxgraph.recently_edited` and `ctxgraph.has_open_pr_passing_ci` for policy rules.

## Latency budget

The brain is on the hot path for policy decisions. Latency must not regress the policy engine.

- p50 ctxgraph query: <50ms
- p95 ctxgraph query: <200ms
- p99 ctxgraph query: <500ms

If a query blows past p99, the policy engine treats it as "no context" and proceeds — never blocks waiting. The face's hover panel can wait longer (>200ms is fine for a hover) but policy rules cannot.

## Failure modes

- **ctxgraph daemon down**: Rhai helpers (`ctxgraph.*`) return `None`; rules that reference ctxgraph fall through to their default behavior. The policy engine keeps working.
- **Schema migration in progress**: ingestion pauses, queries still serve from the old schema. Atomic switchover when migration completes.
- **Disk full**: ingestion drops new events, queries still work. Logged + face notification.
- **Embedding model not loaded**: searches use FTS5 fallback. Slower but functional.

## Phase 3 exit criteria

Detailed in [phases/phase-3-brain.md](../phases/phase-3-brain.md). Top three:

1. Session resumption "open recap" click rate >30% across ≥10 users with ≥1 month of usage.
2. Open-loop nudge dismissal rate <40%.
3. `ctxgraph` GitHub stars / installs grow materially after the layer 3 launch post (the daemon should drive adoption of the underlying library).
