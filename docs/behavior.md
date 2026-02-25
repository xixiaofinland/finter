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

## Session Creation Details

When selecting a folder with no existing session:

1. `tmux new-session -ds <name> -c <path>`
2. `tmux new-window -t <name>:2 -c <path>`
3. `tmux select-window -t <name>:1`
4. switch/attach client

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- Only immediate subdirectories are considered projects.
