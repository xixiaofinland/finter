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
- `src/status.rs`: `status`, `next`, `seen`, `preview`, `copy-mode`, `new-window`
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
| `@finter_remote` | on wrapper panes only: `<ssh alias>:<inner session>`; an empty alias (`:X`, `:X~term`) means the inner server of this machine, so the value is non-empty on every wrapper pane |
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
- Every project session lives on the inner server (`tmux -L finter`) of the machine that owns the project: `X` (Claude) and `X~term` (terminal); windows added with `finter new-window` get `X~term2`, `X~term3`, ... The default server only holds wrappers.
- A local project `X` is shown as wrapper session `X` (no prefix) on the default server.
- Remote project `X` on host `mini` is shown locally as session `mini/X` (tmux accepts `/` in session names).
- Existing default-server sessions (including plain project sessions created by 0.4.x) are shown as-is and keep working.

## Selection UI

- Rows: icon, name, wait time, message.
- Order: asking, done, working (oldest wait first within a state), then existing sessions, then folders. The current session is moved to the front.
- Wrapper sessions (`mini/X`) appear once, as the remote row; inner sessions (`X~term`, ...) never get rows of their own. A local folder row is marked `*` when its project has an inner session.

## Session Creation Details

Selecting a local project: if a default-server session `<name>` exists (an old plain session or an existing wrapper) it is switched to as is. Otherwise a local wrapper is created:

1. `tmux new-session -d -s <name> -c <path> "sh -c 'exec finter attach <name>'"` (window 1)
2. `tmux new-window -d -t <name>: -c <path> "sh -c 'exec finter attach <name> --term'"` (window 2)
3. `@finter_remote=:X` and `:X~term` set on the two panes (empty alias = this machine); no `@finter_colour`
4. select window 1, switch/attach client

Both windows attach to inner sessions `X` and `X~term` on this machine's `tmux -L finter`, creating them if missing; if they already exist (opened from another host over ssh) the wrapper attaches to them, so a project has one session wherever it is opened from.

Remote project without a local wrapper:

1. `tmux new-session -d -s <host>/<X> "ssh -t <alias> finter attach X"` (window 1)
2. `tmux new-window -d -t <host>/<X>: "ssh -t <alias> finter attach X --term"` (window 2)
3. `@finter_remote=<alias>:X` and `<alias>:X~term` set on the two panes, `@finter_colour=colour<N>` on the session
4. select window 1, switch/attach client

`finter attach X [--term[=N]]` (on the remote) resolves X's path from its own roots, writes `inner.conf`, and execs `tmux -L finter -f <conf> new-session -A -s X|X~term|X~termN -c <path>`.

`finter new-window` in a wrapper pane adds a window running `ssh -t <alias> finter attach X --term=N` (local wrapper: `exec finter attach X --term=N`, started in the pane's directory), where N is the lowest terminal slot no wrapper window of X uses (slot 1 is `X~term`), tags it with `@finter_remote=<alias>:X~termN` and selects it. Outside a wrapper it runs a plain `new-window` in the pane's directory.

## Status, Next, Seen

- `status`: local agent panes (default server and inner server) plus every host's cached panes; excludes the current session; asking first (red background, remote colour differs), then done (green), at most 3 names then `+N`, `⚙N` for working, `host ✕` for offline hosts. Prints nothing when idle.
- `next`: oldest asking, else oldest done, across local and remote; switches to the local wrapper (created if missing, for local inner entries too); `X~term[N]` selects the wrapper's terminal window. Without a match it shows `nothing waiting`.
- `seen`: clears `done` for the current session, and with `--leave` also for `#{client_last_session}`, whose `asking` is demoted to `done` (an interrupted permission prompt fires no hook, so leaving is the signal; the `done` reminder stays until the next visit) (used by `client-session-changed`; `pane-focus-in` clears only the current one). Remote sessions are cleared with a fire-and-forget ssh `if-shell` on `tmux -L finter`, and the local cache is updated at once. For a local session the panes in that session on the default server (old plain sessions) and the panes of the project's inner sessions on this machine's `-L finter` server are cleared.

## Known Caveats

- Folder-name collisions across roots can map to the same session name.
- A project whose name ends in `~term` or `~termN` (N >= 2) collides with a terminal session of the project without that suffix.
- Project names containing backslashes are not supported by the remote attach quoting.
- Pane ids (`%3`) are per tmux server and can collide between the default and the inner server; code always pairs a pane id with its socket (`AgentEntry.inner`).
- `split-window` in a wrapper splits the outer wrapper pane, not the project session (see `docs/adr/0001-project-sessions-live-on-the-inner-server.md`).
- `inner.conf` is applied when the inner tmux server starts; changing it later needs a restart of that server (`tmux -L finter kill-server`) to take effect.
- The runtime directory (`$XDG_RUNTIME_DIR/finter`, or `<tmp>/finter-<uid>`) is created with mode 0700.
- Only immediate subdirectories are considered projects.
- `PermissionRequest` means `asking` as soon as the dialog opens; the hook prints nothing on stdout, so it never answers the prompt.
- `Notification` means `asking`, except `notification_type: idle_prompt`, which is ignored so a finished turn stays `done`.
- `Stop` means `done` only when the transcript tail has no unfinished background task (a `toolUseResult` with `backgroundTaskId`, `taskId` or an async `agentId`, without a `<task-notification>` for that id in a user message, queue entry or queued attachment); otherwise the pane stays `working`. A launch older than the 512 KiB tail is not seen.
