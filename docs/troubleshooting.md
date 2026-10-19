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

- `~/.finter.toml` is malformed, or a field has the wrong type.
- The legacy `[ssh]` table is still present. The error says: `the [ssh] table is no longer supported; migrate to [[hosts]]`.

Fix:

- Replace `[ssh]` with a host entry:

```toml
roots = ["/home/username/projects"]

[[hosts]]
name = "mini"
ssh = "mac-mini"
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

## Host shows `mini ✕` or its projects are missing

Cause:

- `ssh <alias> finter list --json` failed within 2 seconds (host off, off the network, key not loaded, or `finter` missing from the remote non-interactive `PATH`).

Fix:

- Run it by hand: `ssh -o BatchMode=yes <alias> finter list --json`.
- Cached data from the last successful refresh is kept and marked offline; it is stored in `$XDG_RUNTIME_DIR/finter/<host>.json`.

## Remote window closes right away

Cause:

- `ssh -t <alias> finter attach <project>` failed (project not in the remote roots, or `tmux` missing there).

Fix:

- Run the same command in a shell to read the error.

## Agent badges never appear

Cause:

- The Claude Code hook is not registered, or `finter` is not on the `PATH` seen by the agent.
- The agent runs outside tmux (`$TMUX_PANE` unset); the hook then does nothing by design.

Fix:

- Check pane options: `tmux show-options -p @agent_state`.
- Pipe sample JSON in: `echo '{"hook_event_name":"Stop"}' | finter hook claude`.

## Canceled picker

Cause:

- Picker aborted (for example pressing `Esc`).

Behavior:

- `finter` exits without switching sessions.
