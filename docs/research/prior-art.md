# Research — Prior art

> What already exists for the problem `homn` is solving, and why none of it adds up to the same thing.

## Permission-notification wrappers

### `agent-notify` (acumino)

[github.com/acumino/agent-notify](https://github.com/acumino/agent-notify) — Get desktop notifications when Claude Code asks for permission.

- **What it does**: Hook → `notify-send` / macOS osascript toast.
- **What it doesn't do**: Decide. Audit. Learn. Persist. Anything stateful.
- **Why we're different**: `homn` *evaluates* policy before it fires anything; the human only sees prompts that fall through to `ask`.

### ntfy.sh integration patterns

Multiple writeups (andrewford.co.nz, martin.hjartmyr.se, alexop.dev) — Claude Code hook → ntfy → phone push.

- **What it does**: Pipes prompts to your phone.
- **What it doesn't do**: Policy, audit, daemon state.
- **Why we're different**: `homn` ships ntfy mirroring **as one surface among many** (TUI / face / phone), behind the same decision pipeline.

### Claude Code Desktop notifications (official)

[Anthropic announcement — Desktop notifications when Claude needs approval.](https://www.threads.com/@boris_cherny/post/DT07_BTk43t)

- **What it does**: Native notification ping when Claude needs you.
- **What it doesn't do**: Decide on your behalf.
- **Why we're different**: Notifications stack and disappear. `homn`'s face is a *persistent state display* you can ignore until something matters; the audit log is the historical record.

## Permission-control / autopilot tools

### Simon Willison's "always allow" / "claudefa.st" fast modes

[claudefa.st — Permission Hook Guide](https://claudefa.st/blog/tools/hooks/permission-hook-guide).

- **What it does**: Hook that auto-approves a curated allowlist of safe tools.
- **What it doesn't do**: Audit. Project-scoped policy. Learning. A face.
- **Why we're different**: `homn`'s Rhai rules express the same allowlist semantically, *plus* every decision is logged with the rule that fired. Provenance, not just behavior.

### doobidoo's Universal Permission Request Hook

[gist.github.com/doobidoo/...](https://gist.github.com/doobidoo/fa84d31c0819a9faace345ca227b268f) — Auto-approve safe MCP tools (integrated into MCP Memory Service).

- **What it does**: Project-specific Python hook that auto-approves listed MCP tools.
- **What it doesn't do**: Generalize across tools, projects, or agents. No audit. No UI.
- **Why we're different**: `homn` is a single daemon + rules engine; you don't write Python hooks per project, you write Rhai rules per project that share infrastructure.

### Dyad's AI-Powered Permission Hooks

[dyad.sh — AI-Powered Claude Code Permission Hooks](https://www.dyad.sh/blog/claude-code-permission-hooks).

- **What it does**: LLM-based judgment per decision (calls Claude/Haiku to evaluate).
- **What it doesn't do**: Be deterministic. Be local. Be debuggable.
- **Why we're different**: `homn` is **deterministic rules first, learning second**. LLM-in-the-loop is a non-goal — it's slow, expensive, and produces decisions you can't reproduce.

## Desktop pets and AI companions

### Claude Code `/buddy` (April Fools 2026)

[mindwiredai.com — Claude Code Buddy: Hatch Your AI Terminal Pet](https://mindwiredai.com/2026/04/06/claude-code-buddy-terminal-pet-guide/) and [smartscope.blog](https://smartscope.blog/en/generative-ai/claude/claude-code-buddy-ai-companion/).

- **What it is**: ASCII Tamagotchi spawned in the Claude Code terminal. Deterministic species per user ID. April Fools easter egg.
- **What it doesn't do**: React to actual agent state. Live outside the terminal. Do anything useful.
- **Why we're different**: `homn`'s face is a *state display*, not a pet. Every animation is triggered by a real event from the daemon's event bus. No idle wandering.

### OpenPets, CodeWalkers, terminalbuddies.com

[openpets.dev](https://openpets.dev/), [terminalbuddies.com](https://terminalbuddies.com/), [DeltaBlade AI companion](https://deltablade.itch.io/ai-a).

- **What they are**: Cute on-screen pixel/Live2D characters that "react" to coding activity.
- **What they don't do**: Connect to policy, audit, or memory. They're decoration.
- **Why we're different**: Same critique — `homn`'s face has utility precisely *because* the daemon underneath is doing real work.

### Open-LLM-VTuber

[github.com/Open-LLM-VTuber](https://github.com/Open-LLM-VTuber/Open-LLM-VTuber) — Live2D + voice + local LLM.

- **What it is**: Conversational anime character with voice.
- **What it doesn't do**: Policy. Coding-agent integration.
- **Why we're different**: We're peripheral signal, not conversational partner.

## Multi-session dashboards

### `claude agents` (official, Nov 2026)

The Claude Code multi-session dashboard view.

- **What it does**: Lists active sessions, lets you switch between them.
- **What it doesn't do**: Tell you *which one needs you* without you looking. Apply policy. Remember.
- **Why we're different**: `homn` complements `claude agents` — the dashboard is a "where are my sessions" view, `homn`'s face is a "what is happening" peripheral.

## Personal-RAG / second-brain tools

### Logseq, Obsidian, Reflect

- **What they are**: Note-taking with linked references.
- **What they don't do**: Ingest events automatically. Live in the dev loop. Be queryable from a policy engine.
- **Why we're different**: `ctxgraph` (layer 3) is an *event-sourced* graph that the daemon writes to; the user doesn't author it. The user *queries* it.

### Cursor's recall / Cline's memory

- **What they are**: Project-scoped memory inside specific IDEs.
- **What they don't do**: Cross-tool. Cross-agent. Cross-machine. Queryable by policy.
- **Why we're different**: `homn`'s memory is *daemon-resident*, accessible from any tool via the MCP server.

## Synthesis — the gap

Every tool above handles one slice. `homn` is the first thing that handles all three (policy + signal + memory) as one coherent product because they share infrastructure:

- All three layers read from the same event bus.
- All three layers persist to the same SQLite-and-ctxgraph storage.
- All three layers are exposed via the same Unix socket + MCP server.
- All three layers degrade gracefully — you can run `homn` headless (policy only), with TUI face, or with full Tauri face.

The unified story is what's defensible. Any individual layer is forkable; the integration isn't.
