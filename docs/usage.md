# Usage

## CLI

| Command | Purpose |
| --- | --- |
| `finter` | Run the picker |
| `finter -d <ABS_PATH> [ABS_PATH ...]` | Save project roots |
| `finter hook claude` | Record Claude Code state in the current tmux pane (stdin: hook JSON) |
| `finter watch -- <command> [args...]` | Run a command, marking the current tmux pane `working` until it ends and `done` after; exits with the command's code |
| `finter status [--current <name>]` | Print the status-bar segment (waiting agents on this machine and hosts); `--current` names the session to leave out, default: ask tmux |
| `finter next` | Jump to the oldest asking agent, then the oldest done one |
| `finter seen [--leave]` | Clear `done` for the current session; with `--leave` also clear the previous one's `done` and demote its `asking` to `done` |
| `finter preview <target>` | Print recent pane output (picker preview) |
| `finter kill` | Switch to the next session and kill the current one; for a wrapper also its inner sessions (on the host, or on this machine for a local project) |
| `finter list [--json]` | Print this machine's projects and agent panes (run over ssh by other hosts) |
| `finter attach <name>` | Attach to (or create) the inner tmux session of a project (run by local wrappers, and over ssh by other hosts) |

## Configure Project Roots

Set one or more absolute root paths where your project folders live:

```bash
finter -d /home/username/projects /home/username/work
```

This writes `~/.finter.toml` and updates the roots.

`~/.finter.toml` format:

```toml
roots = [
    "/home/username/projects",
    "/home/username/work",
]

[[hosts]]
name = "mini"
ssh = "mac-mini"
```

Notes:

- Use absolute paths only.
- `~/...` is not expanded by `finter`.
- Invalid or non-existing paths are ignored at load time.
- `finter -d ...` updates the roots and keeps existing `[[hosts]]` entries.
- Host `name` must be non-empty and free of whitespace, `/`, `:` and `.` (it becomes the session prefix `name/project`).
- Host `color` is optional: an xterm-256 colour index (default `208`, orange) for the host's rows in the picker and `@finter_colour` on its wrapper sessions.
- A legacy `[ssh]` table is rejected with a migration hint.
- Legacy `~/.finter` is no longer used.

## Run Daily

Run `finter` from a shell or tmux popup:

```bash
finter
```

Flow:

1. Reads roots and hosts from `~/.finter.toml`.
2. Finds immediate child directories under each root.
3. Reads current tmux sessions and agent states.
4. Asks each host for `finter list --json` (cached 4 seconds).
5. Shows one list: waiting agents first, then working, then existing sessions, then folders.
6. Select an item to switch/create and attach. A local project opens as a wrapper session `<name>` around the inner sessions on this machine (see below).

Each row shows `icon name wait-time message`:

| Icon | Meaning |
| --- | --- |
| `❓` | agent is asking (needs permission or input) |
| `✅` | agent finished a turn |
| `⚙` | agent is working |
| `*` | session exists, no agent state |
| blank | folder without session |

A project with several tracked panes (Claude, `finter watch`) shows one icon per pane in window order, e.g. `⚙ ✅`; sorting, wait time and message follow the most urgent one.

The right half previews the last lines of the pane (`finter preview`).

## Local Projects

Selecting local project `recap` creates session `recap` (if missing) with a single pane running `finter attach recap`. It attaches to session `recap` on this machine's `tmux -L finter` server, the same one other hosts reach over ssh, so the project has one session, with the same windows, wherever it is opened from. A new inner session starts with window 1 (Claude) and window 2 (terminal); the inner status bar lists its windows. The pane carries `@finter_remote=:recap` (empty alias = this machine). `finter kill` kills the inner session together with the wrapper (plain `kill-session` leaves it running). Plain project sessions created by 0.4.x on the default server keep working.

## Remote Projects

With `[[hosts]] name = "mini"`, remote project `recap` appears as `mini/recap`. Selecting it creates a local session `mini/recap` (if missing) with a single pane running `ssh -t <alias> finter attach recap`. It attaches to session `recap` on a dedicated tmux server (`tmux -L finter`) on the remote machine, so it survives disconnects. The local pane carries `@finter_remote=<alias>:recap`; local project wrappers use the same tag with an empty alias (`:recap`).

Windows and splits of a project live in its inner session. Bind the project keys as shown in [tmux integration](tmux-integration.md) so that `n`, `p`, new window, split, `[` and `1`-`9` act on the inner session from a wrapper; a plain `new-window` in `mini/recap` would open a local shell.

Killing the wrapper with plain `kill-session` leaves the inner session running, and the picker keeps marking `mini/recap` with `*`. `finter kill` ends both: it switches to the next session, kills `recap` on the host over ssh, then kills the wrapper.

## Agent Status

`finter hook claude` maps Claude Code hook events to a state stored in the pane:

| Event | State |
| --- | --- |
| `UserPromptSubmit`, `PreToolUse` | `working` (no write if already working) |
| `PermissionRequest` | `asking` as the dialog opens, message = tool and its main input (`Bash: rm -rf build/`) |
| `Notification` | `asking`, message = notification text (`idle_prompt` is ignored) |
| `Stop` | `done`, message = `last_assistant_message` (transcript tail as fallback, about 60 chars) |
| `Stop` while background work is unfinished | stays `working` (see below) |
| `SessionEnd` | all `@agent_*` options removed |

Claude can end its turn while a background command, async agent or Monitor still runs, for example a CI watch, and is woken when it finishes. On `Stop` the hook reads the transcript tail, and while a background task has no `<task-notification>` yet the pane stays `working`; the `Stop` after the wake-up sets `done`. A background process that never exits (a dev server) keeps the pane `working`.

With `$TMUX_PANE` unset (for example headless `claude -p`) the hook does nothing. It always exits 0 and only writes to stderr on errors.

### Scripts

`finter watch -- <command> [args...]` tracks any long-running command, e.g. `env CLAUDE_CODE_OAUTH_TOKEN=(cat ~/.config/claude-accounts/dev) finter watch -- issue-loop`. It sets the pane to `working` (`@agent=watch`, message = the command line) and, when the command ends, to `done`, whatever the exit code. Ctrl-C reaches the command; `finter watch` survives it to mark `done` and exits with the command's code (128+signal when killed). Without `$TMUX_PANE` it just runs the command, and tmux errors never change the exit code. `finter seen` clears its `done` like Claude's.

`finter status` shows `❓name` (asking, red background), `✅name` (done, green), `⚙N` (working count), at most 3 names then `+N`, and `host ✕` when a host is unreachable. Remote items use other colours. The current session is never badged.
