# Architecture — Data Flow

> End-to-end sequence diagrams for the three things that have to work for `homn` to be useful.

## 1. The happy path: a PermissionRequest answered via face

```
┌──────────────┐  PermissionRequest  ┌──────────────────────┐
│ claude code  ├────────────────────►│ homn hook            │
│ (any session)│  (json on stdin)    │ (subcommand of homn) │
└──────┬───────┘                     └──────────┬───────────┘
       │                                        │ POST /decisions
       │ blocks waiting                         ▼
       │ for hook stdout              ┌─────────────────────┐
       │                              │ homn daemon         │
       │                              │                     │
       │                              │ 1. load policy file │
       │                              │    for this cwd     │
       │                              │ 2. evaluate rules   │
       │                              │    (deny→ask→allow) │
       │                              │ 3. result = ask     │
       │                              │ 4. ctxgraph query   │
       │                              │    (<200ms p95)     │
       │                              │ 5. emit BusEvent:   │
       │                              │    AskOpened        │
       │                              └─────────┬───────────┘
       │                                        │
       │                                        │ broadcast
       │                                        ▼
       │                              ┌─────────────────────┐
       │                              │ homn face (tauri)   │
       │                              │                     │
       │                              │ 6. transitions to   │
       │                              │    ◉ ◉ state        │
       │                              │ 7. renders card C   │
       │                              │    with ctxgraph    │
       │                              │    excerpt          │
       │                              └─────────┬───────────┘
       │                                        │
       │                                        │ user clicks "allow"
       │                                        │
       │                                        │ POST /decisions/:id/answer
       │                                        │   {answer:"allow"}
       │                                        ▼
       │                              ┌─────────────────────┐
       │                              │ homn daemon         │
       │                              │                     │
       │                              │ 8. log to audit.db  │
       │                              │ 9. feed learning    │
       │                              │    subsystem        │
       │                              │ 10. emit BusEvent:  │
       │                              │     AskClosed       │
       │                              └─────────┬───────────┘
       │                                        │
       │  reply to hook                         │
       │  {behavior:"allow"}                    │
       │◄───────────────────────────────────────┘
       │
       ▼
   claude continues with the tool call
```

**Total budget: ≤1.5s p95 from PreToolUse fire to hook return.**

Constituent budgets:
- Policy eval: ≤10ms (Rhai is fast)
- Ctxgraph query: ≤200ms p95
- Bus broadcast → face render: ≤50ms
- Human decision time: unbounded (but Claude hook timeout is 30s; we configure that)
- Audit write + learning update: ≤50ms (off the critical path; can defer)

## 2. The deny path (with PermissionRequest bug workaround)

```
                       Anthropic bug #19298: PermissionRequest hook
                       cannot deny — the prompt still appears.
                       Solution: PTY-tap wrapper that intercepts at the
                       terminal level.

┌──────────────────┐
│ user             │
│ runs:            │
│ homn run claude  │
└────────┬─────────┘
         │ fork+exec with PTY
         ▼
┌──────────────────────────────────────────┐
│ homn PTY wrapper                         │
│                                          │
│ - spawn claude as child                  │
│ - master fd: tap stdout to user's tty    │
│ - regex-match prompt pattern:            │
│   "Do you want to proceed? \(y/n\):"     │
└────────┬─────────────────────────────────┘
         │
         │ (in parallel — also hits the hook path)
         │
         ▼
┌──────────────────┐    POST     ┌─────────────────────┐
│ homn run claude  ├────────────►│ homn daemon         │
│ (PTY tap layer)  │ /decisions  │                     │
└────────┬─────────┘             │ evaluates rules     │
         │                       │ result = DENY       │
         │                       └─────────┬───────────┘
         │                                 │
         │                                 │ broadcast
         │                                 ▼
         │                       ┌─────────────────────┐
         │                       │ homn face / TUI     │
         │                       │ shows "deny" toast  │
         │                       │ (informational —    │
         │                       │  no user input      │
         │                       │  required)          │
         │                       └─────────────────────┘
         │
         │ reply: {decision: "deny"}
         │ arrives within 200ms
         │
         ▼
┌──────────────────────────────────────────┐
│ homn PTY wrapper                         │
│                                          │
│ - writes "n\n" to claude's stdin         │
│   (synthesized keystroke)                │
└────────┬─────────────────────────────────┘
         │
         ▼
┌──────────────────┐
│ claude code      │
│ sees "n" — treats│
│ as user decline  │
│ ► tool call      │
│   aborted        │
└──────────────────┘
```

**Why this works**: the wrapper is racing the human. If the daemon decides `deny` in <200ms, the synthesized `n` lands before the human can type `y`. If the daemon takes longer, the user's interactive prompt is still there as a fallback — no decision is silent.

**Why this is opt-in**: users running `claude` directly (not `homn run claude`) skip the PTY tax. The hook-only path is fine for `allow` decisions, which are most of them.

## 3. Session resumption (Phase 3)

```
┌──────────────┐
│ user runs:   │
│ claude       │
│ (in some cwd)│
└──────┬───────┘
       │ SessionStart hook fires
       ▼
┌──────────────────────────────────────────┐
│ homn hook session-start                  │
│                                          │
│ POSTs to daemon:                         │
│   - session_id                           │
│   - cwd                                  │
└──────────┬───────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────┐
│ homn daemon                              │
│                                          │
│ 1. ctxgraph.session_history(cwd)         │
│    → returns recent sessions + open loops│
│                                          │
│ 2. if any "open loop in this cwd":       │
│    emit BusEvent: SessionResumeOffer     │
└──────────┬───────────────────────────────┘
           │
           │ broadcast
           ▼
┌──────────────────────────────────────────┐
│ homn face                                │
│                                          │
│ shows thought bubble:                    │
│   "Last time here you were drafting      │
│    the Venice email but didn't send it.  │
│    Open recap?"                          │
└──────────┬───────────────────────────────┘
           │ user clicks "Open recap"
           │
           ▼
┌──────────────────────────────────────────┐
│ homn daemon                              │
│                                          │
│ POST UserPromptSubmit hook:              │
│   "Context from last session: ..."       │
│   (pre-pended to user's first prompt)    │
└──────────┬───────────────────────────────┘
           │
           ▼
   claude session starts already oriented
```

## 4. The MCP introspection path

```
agent (claude or other MCP client)
       │
       │ tools/call: query_policy
       │   args: { tool: "Bash", input: { command: "git push --force origin main" } }
       │
       ▼
┌──────────────────────────────────────────┐
│ homn MCP server                          │
│                                          │
│ - look up policy for cwd                 │
│ - eval rules (without logging)           │
│ - return {                               │
│     decision: "deny",                    │
│     rule: "policies/default.rhai:10",    │
│     rule_text: "deny if cmd.matches(...)│
│   }                                      │
└──────────┬───────────────────────────────┘
           │
           ▼
agent uses the answer to decide whether to even attempt the call
```

This is the magic. The agent can ask `homn` *before* committing to an action, learn that it will be denied, and skip the attempt (or propose a different approach). **No existing tool does this.** It's the strongest novelty in the design.

## Critical invariants

- **The policy engine never blocks on the face.** Decision events broadcast; if no subscriber answers in time, the daemon falls back to TUI prompt or ntfy.
- **The face never modifies daemon state.** Only the daemon can mutate audit, policy, or ctxgraph. Face is a strict reader.
- **The audit log is the ground truth.** Every decision lands there before the hook returns. If the daemon crashes between policy eval and audit write, the decision is treated as "no record" and falls back to Claude's default behavior on retry.
- **Ctxgraph queries are best-effort.** Policy rules never fail because ctxgraph is slow or unavailable — they treat missing ctxgraph data as "the condition is false" and continue.
