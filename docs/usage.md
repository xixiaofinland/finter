# Usage

## CLI

| Command | Purpose |
| --- | --- |
| `finter` | Run the picker |
| `finter -d <ABS_PATH> [ABS_PATH ...]` | Save project roots |
| `finter hook claude` | Record Claude Code state in the current tmux pane (stdin: hook JSON) |
| `finter status [--current <name>]` | Print the status-bar segment (waiting agents on this machine and hosts); `--current` names the session to leave out, default: ask tmux |
| `finter next` | Jump to the oldest asking agent, then the oldest done one |
| `finter seen [--leave]` | Clear `done` for the current session; with `--leave` also clear the previous one's `done` and demote its `asking` to `done` |
| `finter preview <target>` | Print recent pane output (picker preview) |
| `finter copy-mode` | Enter copy mode in the inner tmux behind the current wrapper pane |
| `finter new-window` | Open a window; in a wrapper session it is another terminal of the project |
| `finter kill` | Switch to the next session and kill the current one; for a wrapper also its inner sessions (on the host, or on this machine for a local project) |
| `finter list [--json]` | Print this machine's projects and agent panes (run over ssh by other hosts) |
| `finter attach <name> [--term[=N]]` | Attach to the inner tmux session of a project (run by local wrappers, and over ssh by other hosts) |

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

The right half previews the last lines of the pane (`finter preview`).

## Local Projects

Selecting local project `recap` creates session `recap` (if missing) with two windows:

1. `finter attach recap`: Claude window
2. `finter attach recap --term`: terminal window

Both attach to sessions `recap` and `recap~term` on this machine's `tmux -L finter` server, the same ones other hosts reach over ssh, so the project has one session wherever it is opened from. The panes carry `@finter_remote=:recap` / `:recap~term` (empty alias = this machine). `finter new-window` adds `recap~term2`, ... here too, and `finter kill` kills the inner sessions together with the wrapper (plain `kill-session` leaves them running). Plain project sessions created by 0.4.x on the default server keep working.

## Remote Projects

With `[[hosts]] name = "mini"`, remote project `recap` appears as `mini/recap`. Selecting it creates a local session `mini/recap` (if missing) with two windows:

1. `ssh -t <alias> finter attach recap`: Claude window
2. `ssh -t <alias> finter attach recap --term`: terminal window

Both attach to sessions `recap` and `recap~term` on a dedicated tmux server (`tmux -L finter`) on the remote machine, so they survive disconnects. Each local pane carries `@finter_remote=<alias>:<inner session>`; local project wrappers use the same tag with an empty alias (`:recap`).

A plain `new-window` in `mini/recap` opens a local shell. `finter new-window` opens another terminal of the project instead (`recap~term2`, `recap~term3`, ...); bind it as shown in [tmux integration](tmux-integration.md).

Killing the wrapper with plain `kill-session` leaves the inner sessions running, and the picker keeps marking `mini/recap` with `*`. `finter kill` ends both: it switches to the next session, kills `recap`, `recap~term` and any `recap~termN` behind its windows on the host over ssh, then kills the wrapper.

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

`finter status` shows `❓name` (asking, red background), `✅name` (done, green), `⚙N` (working count), at most 3 names then `+N`, and `host ✕` when a host is unreachable. Remote items use other colours. The current session is never badged.
