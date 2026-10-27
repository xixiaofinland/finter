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

# copy mode: wrapper panes (remote or local project) enter copy mode on the inner tmux
bind [ if -F '#{@finter_remote}' 'run-shell -b "finter copy-mode"' 'copy-mode'

# new window: in a wrapper another terminal of the project, else a local one
bind c if -F '#{@finter_remote}' 'run-shell -b "finter new-window"' 'new-window -c "#{pane_current_path}"'

# kill the current session; for a wrapper also its inner sessions
bind x run-shell "finter kill"

# session name in the host's colour for remote projects (local ones have no @finter_colour: white)
set -g status-left "#[fg=#{?@finter_colour,#{@finter_colour},white}][#{session_name}]#[default] "
```

`pane-focus-in` needs `set -g focus-events on`.

Register the hook in Claude Code (`~/.claude/settings.json`) for `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Notification`, `Stop` and `SessionEnd`:

```json
{
  "hooks": {
    "Stop": [
      { "hooks": [{ "type": "command", "command": "finter hook claude" }] }
    ]
  }
}
```

Use the same command for the other events. Project sessions run the hook on the machine that owns them, inside the inner server, so `finter` must be on the `PATH` of that `-L finter` server (it inherits the environment of the first `finter attach`, local or over ssh). Every wrapper pane, local ones included, has a non-empty `@finter_remote` (`:X` for this machine), which the `[`, `c` bindings above test.

## Inner tmux

`finter attach` starts project sessions (local wrappers and remote hosts alike) on `tmux -L finter` with an embedded config (`src/inner.conf`) written to `$XDG_RUNTIME_DIR/finter/inner.conf`: status bar off, no prefix key, mouse on, clipboard passthrough, focus events, `extended-keys` with `csi-u`, vi copy mode (`v`, `C-v`, `V`, `y`). The outer tmux owns all keys; the mouse wheel and PgUp/PgDn reach the inner pane. `split-window` in a wrapper splits the outer wrapper pane, so the new shell is outside the project session.

## Behavior Inside/Outside Tmux

- In tmux, `finter` switches client to the selected session.
- If switch fails, it falls back to `tmux attach -t <session>`.
- If the selected project has no session, `finter` creates its wrapper first (local projects: `X` on the default server, attached to the inner session).

## Per-Project Session Workflow

Use one root per project collection (for example `projects`, `clients`, `experiments`) and keep one tmux session per folder.
The popup picker keeps this fast across many repositories.
