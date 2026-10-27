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
- The status bar shows a short reason: `unreachable`, `timed out`, `finter too old?` (remote finter lacks `list`), or `finter not found`.
- Cached data from the last successful refresh is kept and marked offline; it is stored in `$XDG_RUNTIME_DIR/finter/<host>.json`.

## Remote window closes right away

Cause:

- `ssh -t <alias> finter attach <project>` failed (project not in the remote roots, or `tmux` missing there).

Fix:

- Run the same command in a shell to read the error.

## Old project sessions from 0.4.x are still there

Before this change local projects were plain sessions on the default server. They keep working and still show up in the picker, but they are separate from the project's inner session. When you are done with one, kill it (`tmux kill-session -t <name>`); picking the project again then creates the wrapper `<name>` around the inner session.

## `X~term` sessions or two-window wrappers after upgrading to 0.7

Up to 0.6 every terminal was its own inner session (`X~term`, `X~term2`, ...) and wrappers had one window per terminal. 0.7 has one inner session per project and one-pane wrappers, and does not migrate the old ones. On every machine, once: kill the wrappers (`finter kill` or `tmux kill-session`), run `tmux -L finter kill-server`, then reopen the projects from the picker. Running agents end with the server; resume them with `claude --resume`.

## macOS: `open`, Keychain or clipboard behave differently in a project session

The inner server keeps the environment of the `finter attach` that started it. If it was first started over ssh, its sessions carry that ssh session's environment, so GUI-bound tools (`open`, Keychain, the clipboard) may behave differently than in a session started locally. Restart the inner server from a local shell if that matters: `tmux -L finter kill-server` (this ends all project sessions on this machine).

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
