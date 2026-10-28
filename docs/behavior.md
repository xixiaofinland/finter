# Behavior and Architecture

## High-Level Flow

`finter` is split across:

- `src/main.rs`: CLI parse (clap) + dispatch
- `src/lib.rs`: thin glue: picker flow, `list`, `attach`
- `src/config.rs`: `~/.finter.toml` (`roots`, `roots_config`, `[[hosts]]`), folder scanning
- `src/tmux.rs`: tmux runner for the default and inner (`-L finter`) server, pane listing, pane-option argv builders
- `src/state.rs`: `AgentState`, `AgentEntry`, ordering, `next` selection, age formatting
- `src/hook.rs`: `finter hook claude`
- `src/watch.rs`: `finter watch`
- `src/remote.rs`: `list --json` schema, ssh calls, host cache, `attach`, wrapper-session creation, `inner.conf`
- `src/status.rs`: `status`, `next`, `seen`, `preview`, `kill`
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

Agent-agnostic, written by hooks and `finter watch` and read by `status`, `next`, `seen` and the picker:

| Option | Value |
| --- | --- |
| `@agent` | agent name (`claude`, or `watch` for `finter watch`) |
| `@agent_state` | `working`, `asking` or `done` |
| `@agent_since` | unix epoch seconds of the last state change |
| `@agent_msg` | one-line message (about 60 chars) |
| `@finter_remote` | on wrapper panes only: `<ssh alias>:<inner session>`; an empty alias (`:X`) means the inner server of this machine, so the value is non-empty on every wrapper pane |
| `@finter_colour` | session option on remote wrapper sessions: `colour<N>` from the host's `color`; unset on local wrappers (shown white) |

## `list --json`

Printed by `finter list --json` on each machine (inner server only):

```json
{"v":1,"projects":[{"name":"recap","path":"/p/recap","session_exists":true}],
 "panes":[{"session":"recap","window":"1","pane":"%6","state":"asking","since":1700000000,"msg":"..."}]}
```

`v` is the schema version; a different version is treated as a failed refresh.

## Remote Cache

`ssh -o ConnectTimeout=2 -o BatchMode=yes <alias> finter list --json` is cached in `$XDG_RUNTIME_DIR/finter/<host>.json` (fallback: system temp dir) with a fetch timestamp. Entries older than 4 seconds (30 seconds while the host is offline) are refreshed synchronously by `status` and the picker, bounded by a hard 6 second deadline; ssh also runs with `ServerAliveInterval=2 ServerAliveCountMax=1`. `finter next` never refreshes: it uses the cache as is. A failed refresh keeps the last data, sets the offline flag and stores a short reason (`unreachable`, `timed out`, `finter too old?`, `finter not found`), which `status` shows as `mini ✕ (reason)`. `list --json` is read from the last line that starts with `{`; schema versions >= 1 are accepted and panes with an unknown state are skipped.

## Session Naming Rules

- Session name is based on folder name; `.` and `:` are replaced with `_`.
- Every project is one session `X` on the inner server (`tmux -L finter`) of the machine that owns the project; its windows and panes are the project's terminals (window 1 for Claude, window 2 a terminal when created). The default server only holds wrappers.
- A local project `X` is shown as wrapper session `X` (no prefix) on the default server.
- Remote project `X` on host `mini` is shown locally as session `mini/X` (tmux accepts `/` in session names).
- Existing default-server sessions (including plain project sessions created by 0.4.x) are shown as-is and keep working.

## Selection UI

- Rows: icon, name, wait time, message. A project with several tracked panes shows one icon per pane in window order; the rest of the row uses the most urgent one.
- Order: asking, done, working (oldest wait first within a state), then existing sessions, then folders. The current session is moved to the front.
- Wrapper sessions (`mini/X`) appear once, as the remote row; inner sessions never get rows of their own. A local folder row is marked `*` when its project has an inner session.
- The current row is highlighted across its full width, and moving past the last row wraps to the first (and back).

## Session Creation Details

Selecting a local project: if a default-server session `<name>` exists (an old plain session or an existing wrapper) it is switched to as is. Otherwise a local wrapper is created:

1. `tmux new-session -d -s <name> -c <path> "sh -c 'exec finter attach <name>'"` (one window, one pane)
2. `@finter_remote=:X` set on the pane (empty alias = this machine); no `@finter_colour`
3. switch/attach client

The pane attaches to inner session `X` on this machine's `tmux -L finter`, creating it if missing; if it already exists (opened from another host over ssh) the wrapper attaches to it, so a project has one session, with the same windows, wherever it is opened from.

Remote project without a local wrapper:

1. `tmux new-session -d -s <host>/<X> "ssh -t <alias> finter attach X"` (one window, one pane)
2. `@finter_remote=<alias>:X` set on the pane, `@finter_colour=colour<N>` on the session
3. switch/attach client

`finter attach X` (on the machine owning X) resolves X's path from its own roots and writes `inner.conf`. If inner session `X` is missing it creates it with window 1 and a terminal window 2 in the project directory, then execs `tmux -L finter -f <conf> attach-session -t =X`. All clients of `X` share its current window.

## Status, Next, Seen

- `status`: local agent panes (default server and inner server) plus every host's cached panes; excludes the current session; asking first (red background, remote colour differs), then done (green), at most 3 names then `+N`, `⚙N` for working, `host ✕` for offline hosts. Prints nothing when idle.
- `next`: oldest asking, else oldest done, across local and remote; switches to the local wrapper (created if missing, for local inner entries too) and selects the agent's window and pane in the inner session (over a fire-and-forget ssh for remote projects). Without a match it shows `nothing waiting`.
- `seen`: clears `done` for the current session, and with `--leave` also for `#{client_last_session}`, whose `asking` is demoted to `done` (an interrupted permission prompt fires no hook, so leaving is the signal; the `done` reminder stays until the next visit) (used by `client-session-changed`; `pane-focus-in` clears only the current one). Remote sessions are cleared with a fire-and-forget ssh `if-shell` on `tmux -L finter`, and the local cache is updated at once. For a local session the panes in that session on the default server (old plain sessions) and the panes of the project's inner session on this machine's `-L finter` server are cleared.

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- Project names containing backslashes are not supported by the remote attach quoting.
- Pane ids (`%3`) are per tmux server and can collide between the default and the inner server; code always pairs a pane id with its socket (`AgentEntry.inner`).
- A plain `split-window` or `new-window` in a wrapper acts on the outer wrapper, not the project session; the project-key bindings send `F12` + key to the inner server instead.
- A project is one inner session shared by every machine: `finter kill` on any machine ends it, and the other machines' wrappers close.
- vim-tmux-navigator (`C-h/j/k/l`) is not forwarded: it moves between outer panes, and a wrapper is a single pane.
- `inner.conf` is loaded when the inner tmux server starts and stamps it with `@finter_conf=<finter version>`; `finter attach` re-sources it into a running server whose stamp differs, so an upgrade takes effect without a restart. Settings removed from `inner.conf` stay until `tmux -L finter kill-server`.
- The runtime directory (`$XDG_RUNTIME_DIR/finter`, or `<tmp>/finter-<uid>`) is created with mode 0700.
- Only immediate subdirectories are considered projects.
- `PermissionRequest` means `asking` as soon as the dialog opens; the hook prints nothing on stdout, so it never answers the prompt.
- `Notification` means `asking`, except `notification_type: idle_prompt`, which is ignored so a finished turn stays `done`.
- `Stop` means `done` only when the transcript tail has no unfinished background task (a `toolUseResult` with `backgroundTaskId`, `taskId` or an async `agentId`, without a `<task-notification>` for that id in a user message, queue entry or queued attachment); otherwise the pane stays `working`. A launch older than the 512 KiB tail is not seen.
- A `Stop` in a pane the user is looking at clears the pane instead of setting `done`: the pane is the active pane of its session's current window and a client of that session has the `focused` flag. A wrapper's client on the inner server gets focus from the outer tmux (`focus-events`), so it loses it when the user switches session or the terminal loses focus. No focus change follows a turn ending in view, so `finter seen` would never clear that `done`.
