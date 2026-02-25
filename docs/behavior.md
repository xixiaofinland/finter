# Behavior and Architecture

## High-Level Flow

`finter` is split across:

- `src/main.rs`: CLI parse + dispatch
- `src/lib.rs`: core behavior

Runtime path:

1. Parse args with `clap`.
2. If `-d/--directory` is provided, save/update roots in `~/.finter.toml`.
3. Otherwise:
   - load configured roots and ssh settings
   - scan immediate child directories
   - load tmux sessions
   - build picker items
   - select item with `skim`
   - switch/create tmux session

## Core Functions

- `save_paths(args)`: writes/updates roots in `~/.finter.toml`
- `load_config()`: reads and parses `~/.finter.toml`
- `load_project_paths(...)`: validates root paths
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
- Configured SSH session (`ssh.session_name`) is always included and pinned near the top.

## Session Creation Details

When selecting a folder with no existing session:

1. `tmux new-session -ds <name> -c <path>`
2. `tmux new-window -t <name>:2 -c <path>`
3. `tmux select-window -t <name>:1`
4. switch/attach client

When selecting configured SSH session with no existing session:

1. `tmux new-session -ds <ssh.session_name> -c <home>`
2. `tmux send-keys -t <ssh.session_name>:1 "<ssh connect command>" C-m`
3. switch/attach client

If configured SSH session already exists, `finter` only switches/attaches and does not send additional commands.

`<ssh connect command>` is:

- `ssh <ssh.primary>`
- with `ssh.tailscale` set: `if nc -z -w2 192.168.1.200 22; then ssh <ssh.primary>; else ssh <ssh.tailscale>; fi`

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- Only immediate subdirectories are considered projects.
