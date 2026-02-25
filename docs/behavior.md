# Behavior and Architecture

## High-Level Flow

`finter` is split across:

- `src/main.rs`: CLI parse + dispatch
- `src/lib.rs`: core behavior

Runtime path:

1. Parse args with `clap`.
2. If `-d/--directory` is provided, save roots to `~/.finter`.
3. Otherwise:
   - load configured roots
   - scan immediate child directories
   - load tmux sessions
   - build picker items
   - select item with `skim`
   - switch/create tmux session

## Core Functions

- `save_paths(args)`: writes roots to `~/.finter`
- `load_project_paths()`: reads and validates root paths
- `get_folders(paths)`: lists folders under roots
- `get_sessions()`: reads `tmux list-sessions -F #S`
- `build_projects(...)`: merges folder list + existing sessions
- `select_in_skim(...)`: interactive fuzzy selection
- `run_finter()`: orchestration + tmux actions

## Session Naming Rules

- New session name is based on folder name.
- `.` and `:` are replaced with `_`.
- Existing sessions are shown as-is.

## Selection UI

- Existing tmux sessions are prefixed with `*` in display.
- New folder sessions are prefixed with a space.
- Built-in `ssh_mac_mini` is always included and pinned near the top.

## Session Creation Details

When selecting a folder with no existing session:

1. `tmux new-session -ds <name> -c <path>`
2. `tmux new-window -t <name>:2 -c <path>`
3. `tmux select-window -t <name>:1`
4. switch/attach client

When selecting `ssh_mac_mini` with no existing session:

1. `tmux new-session -ds ssh_mac_mini -c <home>`
2. `tmux send-keys -t ssh_mac_mini:1 "<ssh connect command>" C-m`
3. switch/attach client

If `ssh_mac_mini` already exists, `finter` only switches/attaches and does not send additional commands.

`<ssh connect command>` is:

- `ssh xixiao@192.168.1.200` by default
- with `FINTER_SSH_TAILSCALE_TARGET` set: `if nc -z -w2 192.168.1.200 22; then ssh xixiao@192.168.1.200; else ssh <tailscale_target>; fi`

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- Only immediate subdirectories are considered projects.
