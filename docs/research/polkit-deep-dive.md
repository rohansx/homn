# Research — polkit deep dive

> The architectural pattern we're borrowing. Read this once; the rest of `homn` makes more sense afterwards.

## What polkit actually is

Polkit is **not** an auth UI. It's a **policy decision separated from policy enforcement and from human interaction**. Three roles, three processes:

| Role                | Process                                       | Responsibility                                                    |
|---------------------|-----------------------------------------------|-------------------------------------------------------------------|
| Decision authority  | `polkitd` (system daemon, D-Bus service `org.freedesktop.PolicyKit1`) | Owns the policy: *can subject X do action Y?* Reads .policy XML + JS rules. |
| Enforcement point   | A privileged tool that asks `polkitd` (e.g. `pkexec`, `systemctl`, NetworkManager) | "Should I let this happen?" Calls polkitd over D-Bus, gates the action on the answer. |
| Authentication agent| Per-session user-space UI (`polkit-gnome-authentication-agent-1`, `hyprpolkitagent`, `lxqt-policykit-agent`, etc.) | Renders the password dialog when polkitd says *"prove you're you"*. Registers with polkitd at session start. |

The split is the whole insight. The decision authority doesn't know how to draw on your screen. The enforcement point doesn't know your policy. The auth agent doesn't know the rules. **Each is replaceable.**

## How `pkexec` actually pops up

```
$ pkexec systemctl restart nginx
       │
       ▼
pkexec  ─── D-Bus call ────► polkitd
                              │
                              │ evaluate policy:
                              │   action = org.freedesktop.systemd1.manage-units
                              │   rule = "needs auth as admin, persistent for 5 min"
                              │
                              ▼
                            polkitd asks the agent registered for this session:
                              │
                              ▼
                            polkit-gnome-authentication-agent-1
                              │ renders password dialog (it picks the surface — X11/Wayland, Hyprland layer-shell, whatever)
                              │
                              ▼
                            user types password / clicks OK
                              │
                              ▼
                            agent ─── D-Bus reply ──► polkitd ─── reply ──► pkexec ─── runs the command
```

Critical detail: **`pkexec` doesn't draw the popup itself.** It can't even — it's a CLI tool. It asks `polkitd`, which finds whatever agent the user's session registered with at login, and that agent picks the rendering surface.

If no graphical agent is registered, `pkexec` registers its own **textual** fallback agent. This is exactly the pattern we'll borrow for SSH-only sessions: degrade gracefully to a terminal prompt.

## Why this matters for homn

Map the polkit roles onto our system:

| Polkit                                  | homn                                                                     |
|-----------------------------------------|--------------------------------------------------------------------------|
| `polkitd` (decision authority)          | `homn daemon` — evaluates Rhai rules, owns the audit log                 |
| `pkexec` / NetworkManager (enforcement) | `homn` Claude Code hook + PTY-tap fallback — gates tool calls            |
| polkit-gnome-agent / hyprpolkitagent    | `homn face` (Tauri window) OR TUI prompt in the calling terminal         |
| .policy XML + JS rules                  | `~/.config/homn/policies/*.rhai`                                         |
| pkaction / pkcheck                      | `homn rule` / `homn log` CLI + MCP server                                |

The decision authority knows nothing about how the decision is surfaced. The surface (TUI / face / phone) knows nothing about the rules. **Same separation, different domain.**

## Wayland-specific notes (matters because most modern Linux is Wayland)

Polkit auth agents on Wayland are notoriously fragile because:

1. **No window-manager-agnostic always-on-top.** X11 had `_NET_WM_STATE_ABOVE`; Wayland deliberately doesn't expose anything equivalent to non-privileged clients. Each compositor provides its own protocol (`wlr-layer-shell` for Hyprland/Sway/river, KDE has KWayland, GNOME has nothing useful).
2. **Compositor-specific agent setup.** GNOME bundles its agent; Hyprland needs `hyprpolkitagent` explicitly added to `exec-once`. If no agent is installed, `pkexec` silently falls back to the TTY prompt — fine for SSH, broken for desktop.
3. **No cross-process window introspection.** A Wayland client cannot ask the compositor *"where on screen is the terminal that needs this prompt?"* — by design. This kills the "face points at the session" idea for v0 (see [docs/risks/known-unknowns.md](../risks/known-unknowns.md)).

Implication for `homn face`: build on `wlr-layer-shell` where available (Hyprland/Sway), fall back to a regular xdg-toplevel "always on top hint" elsewhere, fall back to TUI prompt if neither works.

## What we're NOT borrowing

Polkit has 20 years of organizational baggage we should skip:

- **D-Bus.** Polkit needs D-Bus because it spans multiple security domains. `homn` is a single-user daemon — Unix socket is simpler, faster, and easier to debug (`socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/homn.sock`).
- **.policy XML files.** Verbose, hard to read, written by package maintainers not users. We use Rhai because the user *is* the policy author.
- **System-wide installation.** Polkit is `/usr/lib/polkit-1/` and root-owned. `homn` is per-user, no privileged install, no setuid bits.
- **`pkttyagent`'s prompt format.** We can do better — the TUI prompt in v0 is closer to a fish/zsh-style colored prompt than polkit's ASCII separators.

## Open questions polkit raises

- **Multi-session.** Polkit ties an auth decision to a session UID + login session ID. `homn` needs the same: a Claude Code session ID → which face/TUI gets the prompt. Hook payload includes session ID; we plumb it.
- **Decision caching ("keep authorized for 5 min").** Polkit has a notion of *persistent* authorization within a session. `homn`'s equivalent is the learning layer: after N consistent allows, the user is offered a rule that makes future asks unnecessary. Different mechanism, same UX.
- **Privilege escalation.** Polkit's whole reason for existing is "let an unprivileged user do a privileged thing safely." `homn` does **not** elevate privileges; it gates calls that Claude Code can already make. This means our threat model is simpler — we're an *allowlist*, not a *grant*.

## Sources

- [polkit Reference Manual — pkexec(1)](https://www.freedesktop.org/software/polkit/docs/latest/pkexec.1.html)
- [Arch Wiki — polkit](https://wiki.archlinux.org/title/Polkit)
- [Hyprland Wiki — hyprpolkitagent](https://wiki.hypr.land/Hypr-Ecosystem/hyprpolkitagent/)
- elvinguti.dev — [Fix missing graphical PolicyKit Authentication agent](https://elvinguti.dev/posts/fix-missing-polkit/)
