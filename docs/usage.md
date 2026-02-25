# Usage

## CLI

`finter` has two modes:

- Configure roots: `finter -d <ABS_PATH> [ABS_PATH ...]`
- Run picker: `finter`

## Configure Project Roots

Set one or more absolute root paths where your project folders live:

```bash
finter -d /home/username/projects /home/username/work
```

This writes `~/.finter.toml` and updates the `roots` field.

`~/.finter.toml` format:

```toml
roots = [
    "/home/username/projects",
    "/home/username/work",
]

[ssh]
session_name = "ssh_mac_mini"
primary = "user@192.168.1.200"
# optional tailscale fallback
# tailscale = "user@macmini.tailnet.ts.net"
```

Notes:

- Use absolute paths only.
- `~/...` is not expanded by `finter`.
- Invalid or non-existing paths are ignored at load time.
- `finter -d ...` updates `roots` and keeps existing `[ssh]` settings.
- Legacy `~/.finter` is no longer used.

## Run Daily

Run `finter` from a shell or tmux popup:

```bash
finter
```

Flow:

1. Reads roots and SSH settings from `~/.finter.toml`.
2. Finds immediate child directories under each root.
3. Reads current tmux sessions.
4. Adds built-in item `ssh_mac_mini`.
5. Shows combined list in fuzzy picker.
6. Select item to switch/create and attach.

Configured SSH session behavior:

- If session exists, `finter` just switches/attaches.
- If session does not exist, `finter` creates a one-window session and sends:

```bash
ssh <ssh.primary>
```

- Optional Tailscale fallback in TOML:

```toml
[ssh]
tailscale = "user@macmini.tailnet.ts.net"
```

When `ssh.tailscale` is set, `finter` sends a command that tries LAN first and falls back to this target only if `192.168.1.200:22` is unreachable.

## Typical Tmux Workflow

1. Configure roots once with `-d`.
2. Bind a tmux key to `display-popup -E "finter"`.
3. Hit hotkey, select folder, continue work in that session.
