# Behavior and Architecture

## High-Level Flow

`finter` is split across:

- `src/main.rs`: CLI parse (clap) + dispatch
- `src/lib.rs`: thin glue: picker flow, `list`, `attach`
- `src/config.rs`: `~/.finter.toml` (`roots`, `roots_config`, `[[hosts]]`), folder scanning
- `src/tmux.rs`: tmux runner for the default and inner (`-L finter`) server, pane listing, pane-option argv builders
- `src/state.rs`: `AgentState`, `AgentEntry`, ordering, `next` selection, age formatting
- `src/hook.rs`: `finter hook claude`
- `src/remote.rs`: `list --json` schema, ssh calls, host cache, `attach`, wrapper-session creation, `inner.conf`
- `src/status.rs`: `status`, `next`, `seen`, `preview`, `copy-mode`
- `src/picker.rs`: picker rows and the skim UI

Runtime path of the picker:

1. Parse args with `clap`.
2. If `-d/--directory` is provided, save/update roots in `~/.finter.toml`.
3. Otherwise:
   - load roots and hosts
   - scan immediate child directories
   - load tmux sessions and local agent panes
   - load each host's state (`finter list --json` over ssh, cached 4 seconds)
   - build rows, select with `skim` (preview = `finter preview`)
   - switch/create tmux session

## Pane Options Contract

Agent-agnostic, written by hooks and read by `status`, `next`, `seen` and the picker:

| Option | Value |
| --- | --- |
| `@agent` | agent name (`claude`) |
| `@agent_state` | `working`, `asking` or `done` |
| `@agent_since` | unix epoch seconds of the last state change |
| `@agent_msg` | one-line message (about 60 chars) |
| `@finter_remote` | on wrapper panes only: `<ssh alias>:<inner session>` |

## `list --json`

Printed by `finter list --json` on each machine (inner server only):

```json
{"v":1,"projects":[{"name":"recap","path":"/p/recap","session_exists":true}],
 "panes":[{"session":"recap","window":"1","pane":"%6","state":"asking","since":1700000000,"msg":"..."}]}
```

`v` is the schema version; a different version is treated as a failed refresh.

## Remote Cache

`ssh -o ConnectTimeout=2 -o BatchMode=yes <alias> finter list --json` is cached in `$XDG_RUNTIME_DIR/finter/<host>.json` (fallback: system temp dir) with a fetch timestamp. Entries older than 4 seconds are refreshed synchronously, bounded by a hard 6 second deadline. A failed refresh keeps the last data and sets the offline flag.

## Session Naming Rules

- New local session name is based on folder name; `.` and `:` are replaced with `_`.
- Remote project `X` on host `mini` is shown locally as session `mini/X` (tmux accepts `/` in session names).
- On the remote machine the inner sessions are `X` (Claude) and `X~term` (terminal) on `tmux -L finter`.
- Existing sessions are shown as-is.

## Selection UI

- Rows: icon, name, wait time, message.
- Order: asking, done, working (oldest wait first within a state), then existing sessions, then folders. The current session is moved to the front.
- Wrapper sessions (`mini/X`) appear once, as the remote row.

## Session Creation Details

Local project without a session:

1. `tmux new-session -ds <name> -c <path>` (creates window 1)
2. `tmux new-window -t <name>:2 -c <path>` (creates window 2)
3. `tmux select-window -t <name>:1` (keeps window 1 active)
4. switch/attach client

Remote project without a local wrapper:

1. `tmux new-session -d -s <host>/<X> "ssh -t <alias> finter attach X"` (window 1)
2. `tmux new-window -d -t <host>/<X>: "ssh -t <alias> finter attach X --term"` (window 2)
3. `@finter_remote=<alias>:X` and `<alias>:X~term` set on the two panes
4. select window 1, switch/attach client

`finter attach X [--term]` (on the remote) resolves X's path from its own roots, writes `inner.conf`, and execs `tmux -L finter -f <conf> new-session -A -s X|X~term -c <path>`.

## Status, Next, Seen

- `status`: local agent panes plus every host's cached panes; excludes the current session; asking first (red background, remote colour differs), then done (green), at most 3 names then `+N`, `⚙N` for working, `host ✕` for offline hosts. Prints nothing when idle.
- `next`: oldest asking, else oldest done, across local and remote; switches to the local wrapper (created if missing); `X~term` selects the wrapper's terminal window. Without a match it shows `nothing waiting`.
- `seen`: clears `done` (never `asking`) for the current session and `#{client_last_session}`. Remote sessions are cleared with a fire-and-forget ssh `if-shell` on `tmux -L finter`, and the local cache is updated at once.

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- Only immediate subdirectories are considered projects.
- `Notification` means `asking`, except `notification_type: idle_prompt`, which is ignored so a finished turn stays `done`.
