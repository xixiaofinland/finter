# Usage

## CLI

| Command | Purpose |
| --- | --- |
| `finter` | Run the picker |
| `finter -d <ABS_PATH> [ABS_PATH ...]` | Save project roots |
| `finter hook claude` | Record Claude Code state in the current tmux pane (stdin: hook JSON) |
| `finter status [--current <name>]` | Print the status-bar segment (waiting agents on this machine and hosts); `--current` names the session to leave out, default: ask tmux |
| `finter next` | Jump to the oldest asking agent, then the oldest done one |
| `finter seen [--leave]` | Clear `done` for the current session; with `--leave` also the previous one |
| `finter preview <target>` | Print recent pane output (picker preview) |
| `finter copy-mode` | Enter copy mode in the remote tmux behind the current pane |
| `finter list [--json]` | Print this machine's projects and agent panes (run over ssh by other hosts) |
| `finter attach <name> [--term]` | Attach to the inner tmux session of a project (run over ssh by other hosts) |

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
6. Select an item to switch/create and attach.

Each row shows `icon name wait-time message`:

| Icon | Meaning |
| --- | --- |
| `❓` | agent is asking (needs permission or input) |
| `✅` | agent finished a turn |
| `⚙` | agent is working |
| `*` | session exists, no agent state |
| blank | folder without session |

The right half previews the last lines of the pane (`finter preview`).

## Remote Projects

With `[[hosts]] name = "mini"`, remote project `recap` appears as `mini/recap`. Selecting it creates a local session `mini/recap` (if missing) with two windows:

1. `ssh -t <alias> finter attach recap`: Claude window
2. `ssh -t <alias> finter attach recap --term`: terminal window

Both attach to sessions `recap` and `recap~term` on a dedicated tmux server (`tmux -L finter`) on the remote machine, so they survive disconnects. Each local pane carries `@finter_remote=<alias>:<inner session>`.

## Agent Status

`finter hook claude` maps Claude Code hook events to a state stored in the pane:

| Event | State |
| --- | --- |
| `UserPromptSubmit`, `PreToolUse` | `working` (no write if already working) |
| `Notification` | `asking`, message = notification text (`idle_prompt` is ignored) |
| `Stop` | `done`, message = last assistant text (about 60 chars) |
| `SessionEnd` | all `@agent_*` options removed |

With `$TMUX_PANE` unset (for example headless `claude -p`) the hook does nothing. It always exits 0 and only writes to stderr on errors.

`finter status` shows `❓name` (asking, red background), `✅name` (done, green), `⚙N` (working count), at most 3 names then `+N`, and `host ✕` when a host is unreachable. Remote items use other colours. The current session is never badged.
