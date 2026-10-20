# Tmux Integration

## Recommended Binding

Default example:

```tmux
bind C-o display-popup -E "finter"
```

This means `Prefix + C-o` opens a popup and runs `finter`.

## Custom Key

Use any key you prefer:

```tmux
bind C-p display-popup -E "finter"
```

## Popup Options

You can tune popup size and position:

```tmux
bind C-o display-popup -w 80% -h 80% -E "finter"
```

## Agent Status Wiring

```tmux
# status bar: waiting agents (empty when nothing is waiting)
set -g status-right "#(finter status --current '#{session_name}') %H:%M"
set -g status-interval 5

# jump to the oldest asking agent, then the oldest done one
bind a run-shell -b "finter next"

# clear `done` on the session you enter; on a session switch also on the one you leave
set-hook -g client-session-changed 'run-shell -b "finter seen --leave"'
set-hook -g pane-focus-in 'run-shell -b "finter seen"'

# copy mode: panes that show a remote session enter copy mode on the remote tmux
bind [ if -F '#{@finter_remote}' 'run-shell -b "finter copy-mode"' 'copy-mode'
```

`pane-focus-in` needs `set -g focus-events on`.

Register the hook in Claude Code (`~/.claude/settings.json`) for `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Notification`, `Stop` and `SessionEnd`:

```json
{
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command", "command": "finter hook claude" }] }]
  }
}
```

Use the same command for the other events. Remote sessions run the hook on the remote machine, so `finter` must be on the `PATH` of the remote tmux server (it inherits the environment of the first `finter attach`).

## Inner tmux

`finter attach` starts the remote sessions on `tmux -L finter` with an embedded config (`src/inner.conf`) written to `$XDG_RUNTIME_DIR/finter/inner.conf`: status bar off, no prefix key, mouse on, clipboard passthrough, `extended-keys` with `csi-u`, vi copy mode (`v`, `C-v`, `V`, `y`). The outer tmux owns all keys; the mouse wheel and PgUp/PgDn reach the remote pane.

## Behavior Inside/Outside Tmux

- In tmux, `finter` switches client to the selected session.
- If switch fails, it falls back to `tmux attach -t <session>`.
- If selected session does not exist, `finter` creates it first.

## Per-Project Session Workflow

Use one root per project collection (for example `projects`, `clients`, `experiments`) and keep one tmux session per folder.
The popup picker keeps this fast across many repositories.
