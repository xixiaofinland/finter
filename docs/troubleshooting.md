# Troubleshooting

## `finter: command not found`

Cause:

- Binary is not installed or not on `PATH`.

Fix:

```bash
cargo install finter
```

Ensure Cargo bin directory is on `PATH` (commonly `~/.cargo/bin`).

## `Err: no valid path is configured.`

Cause:

- `~/.finter.toml` is missing, has empty `roots`, or all configured roots are invalid.

Fix:

```bash
finter -d /absolute/path/one /absolute/path/two
```

## `Err: invalid TOML config ...`

Cause:

- `~/.finter.toml` is malformed, or required fields are missing.

Fix:

- Ensure config includes both `roots` and `[ssh]`:

```toml
roots = ["/home/username/projects"]

[ssh]
session_name = "ssh_mac_mini"
primary = "user@192.168.1.200"
```

## `Err: no folder is found in the configured paths.`

Cause:

- Configured roots exist but have no subdirectories.

Fix:

- Point `-d` to roots that contain project folders.

## Popup opens but session switch fails

Cause:

- tmux client/session state mismatch.

Behavior:

- `finter` tries `switch-client` then falls back to `attach`.

Fix:

- Verify tmux is running and session name exists:

```bash
tmux list-sessions
```

## SSH session does not use Tailscale fallback

Cause:

- `ssh.tailscale` is missing or empty in `~/.finter.toml`.

Fix:

- Set `ssh.tailscale` in `~/.finter.toml`, for example:

```toml
[ssh]
tailscale = "user@macmini.tailnet.ts.net"
```

- Restart tmux server if the popup still uses stale config/environment.

## Canceled picker

Cause:

- Picker aborted (for example pressing `Esc`).

Behavior:

- `finter` exits without switching sessions.
