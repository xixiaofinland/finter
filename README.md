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

- Reads configured roots from `~/.finter` (one absolute path per line).
- Scans immediate child directories of those roots.
- Lists those folders together with existing tmux session names.
- Uses a popup-compatible fuzzy selector (`skim`).
- On select:
  - existing session: switch/attach to it
  - missing session: create detached session in folder, create a second window, then switch/attach

## Docs

- [Usage](docs/usage.md)
- [Tmux Integration](docs/tmux-integration.md)
- [Behavior and Architecture](docs/behavior.md)
- [Troubleshooting](docs/troubleshooting.md)
