# Finter

`finter` is a small Rust CLI for tmux users who work across many project folders.
It lets you open a popup picker, fuzzy-find a folder, and jump into a matching tmux
session (or create it if missing).

![screenshot](https://github.com/xixiaofinland/finter/blob/master/pic/1.png)

## Install

You need [Cargo](https://www.rust-lang.org/tools/install):

```bash
cargo install finter
```

## Quick Start

1. Configure project roots (absolute paths only):

```bash
finter -d /home/username/projects /home/username/work
```

2. Add a tmux popup hotkey (default example: `Prefix + C-o`):

```tmux
bind C-o display-popup -E "finter"
```

3. Trigger the hotkey, fuzzy-select a folder, and `finter` will switch/create the tmux session.

## What It Does

- Reads roots and remote hosts from `~/.finter.toml`.
- Scans immediate child directories of those roots.
- Lists those folders together with existing tmux sessions and, for each `[[hosts]]` entry, that machine's projects as `<host>/<project>`.
- Shows which coding agents (Claude Code) are asking, done or working, with wait time and a one-line message, and previews the pane.
- Uses a popup-compatible fuzzy selector (`skim`).
- On select:
  - existing session: switch/attach to it
  - missing local project session: create wrapper session `<project>` with one pane attached to the project's session on this machine's inner tmux server (`tmux -L finter`), which starts with a Claude window and a terminal window, then switch/attach
  - remote project: create local session `<host>/<project>` with one pane attached to the project's session on the remote machine's inner server, then switch/attach

Default `~/.finter.toml` (created by `finter -d ...`):

```toml
roots = [
    "/home/username/projects",
    "/home/username/work",
]

# Optional: one entry per remote machine.
[[hosts]]
name = "mini"
ssh = "mac-mini"
color = 180 # optional xterm-256 colour for this host's sessions, default 208
```

You can also copy the repo template: `.finter.toml.example`.

Notes:

- `ssh` is an alias from `~/.ssh/config` (or `user@host`); the remote needs `finter` on its non-interactive `PATH` and the same `~/.finter.toml` roots.
- `color` tints the host's rows in the picker and is stored as `@finter_colour` on its wrapper sessions; see `docs/tmux-integration.md` for the status-bar line that reads it.
- The legacy `[ssh]` table is no longer supported: finter refuses to start and tells you to migrate to `[[hosts]]`.
- Legacy `~/.finter` path-list config is no longer read.

## Agent Status

`finter hook claude` records what Claude Code is doing in the tmux pane it runs in (pane options `@agent`, `@agent_state`, `@agent_since`, `@agent_msg`). `finter status` renders the waiting agents for the tmux status bar, `finter next` jumps to the oldest one, and `finter seen` clears `done` badges. See [Tmux Integration](docs/tmux-integration.md) for the hook and tmux wiring.

Long-running scripts can be tracked the same way, next to Claude in another window of the project. `finter watch -- <command> [args...]` runs the command (exit code passed through) and shows its pane as working until it ends, then done, whatever the exit code:

```fish
env CLAUDE_CODE_OAUTH_TOKEN=(cat ~/.config/claude-accounts/dev) finter watch -- issue-loop
```

The picker row shows one icon per tracked pane in window order, e.g. `⚙ ✅ finter` for Claude working in window 1 and a finished script in window 2.

## Docs

- [Usage](docs/usage.md)
- [Tmux Integration](docs/tmux-integration.md)
- [Behavior and Architecture](docs/behavior.md)
- [Troubleshooting](docs/troubleshooting.md)
