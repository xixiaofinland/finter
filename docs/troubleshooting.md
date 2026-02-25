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

- `~/.finter` is missing, empty, or contains no valid directories.

Fix:

```bash
finter -d /absolute/path/one /absolute/path/two
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

## `ssh_mac_mini` does not use Tailscale fallback

Cause:

- `FINTER_SSH_TAILSCALE_TARGET` is not set in the environment where tmux launches `finter`.

Fix:

- Export the variable before starting tmux, for example:

```bash
export FINTER_SSH_TAILSCALE_TARGET="xixiao@macmini.tailnet.ts.net"
```

- Restart tmux server if needed so popup commands inherit updated environment.

## Canceled picker

Cause:

- Picker aborted (for example pressing `Esc`).

Behavior:

- `finter` exits without switching sessions.
