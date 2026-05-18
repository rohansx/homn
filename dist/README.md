# dist/ — packaging artifacts

Templates and unit files for running `homn` as a long-lived background process.

## `homn.service` — systemd user unit (Linux)

Install + enable:

```sh
mkdir -p ~/.config/systemd/user
cp dist/homn.service ~/.config/systemd/user/homn.service

# Edit ExecStart if your `homn` binary isn't at ~/.cargo/bin/homn:
sed -i "s|%h/.cargo/bin/homn|$(which homn)|" ~/.config/systemd/user/homn.service

systemctl --user daemon-reload
systemctl --user enable --now homn

# Verify:
systemctl --user status homn
journalctl --user -u homn -f
```

The unit:

- Restarts on failure with a 2s backoff.
- Caps memory at 256 MB and tasks at 64 (a single-user daemon never needs more).
- Locks down outbound network access to localhost only (loosen if you enable the optional
  ntfy mirror in `homn.toml`).
- Read-only access to `$HOME` except for `~/.config/homn`, `~/.local/share/homn`, and the
  runtime dir (`$XDG_RUNTIME_DIR`).
- Standard systemd hardening (`NoNewPrivileges`, `ProtectSystem=strict`,
  `ProtectKernelTunables`, etc.).

Disable + remove:

```sh
systemctl --user disable --now homn
rm ~/.config/systemd/user/homn.service
```

## macOS launchd plist

TODO — coming in T081 polish. For now on macOS, run `homn daemon --foreground` in a tmux
pane or under a `launchctl` wrapper of your own.
