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

# project keys: in a wrapper pane they act on the project's inner session (sent as
# F12 + key, see Inner tmux below), elsewhere on the outer tmux as usual
bind n if -F '#{@finter_remote}' 'send-keys F12 n' 'next-window'
bind p if -F '#{@finter_remote}' 'send-keys F12 p' 'previous-window'
bind c if -F '#{@finter_remote}' 'send-keys F12 =' 'new-window -c "#{pane_current_path}"'
bind l if -F '#{@finter_remote}' 'send-keys F12 l' 'split-window -h -c "#{pane_current_path}"'
bind q if -F '#{@finter_remote}' 'send-keys F12 q' 'kill-pane'
bind [ if -F '#{@finter_remote}' 'send-keys F12 [' 'copy-mode'
bind 1 if -F '#{@finter_remote}' 'send-keys F12 1' 'select-window -t :=1'
# ... and so on for 2-9

# a wrapper is one window; its project's windows show in the inner status bar instead
set -g window-status-format '#{?@finter_remote,,#I_#W}'
set -g window-status-current-format '#{?@finter_remote,,#I_#W}'

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

Use the same command for the other events. Project sessions run the hook on the machine that owns them, inside the inner server, so `finter` must be on the `PATH` of that `-L finter` server (it inherits the environment of the first `finter attach`, local or over ssh). Every wrapper pane, local ones included, has a non-empty `@finter_remote` (`:X` for this machine), which the project-key bindings above test.

## Inner tmux

`finter attach` starts project sessions (local wrappers and remote hosts alike) on `tmux -L finter` with an embedded config (`src/inner.conf`) written to `$XDG_RUNTIME_DIR/finter/inner.conf`: a status bar listing only the project's windows, no prefix key, `base-index 1`, mouse on, clipboard passthrough, focus events, `extended-keys` with `csi-u`, vi copy mode (`v`, `C-v`, `V`, `y`). A project is one inner session; its windows and panes are the project's terminals, and every machine's wrapper shows the same ones (see `docs/adr/0002-one-inner-session-per-project.md`).

The outer tmux owns the prefix. `F12` puts the inner server into its `finter` key table, where `n`, `p`, `=` (new window), `l` (split), `q` (kill pane), `[` (copy mode) and `1`-`9` act on the project; the outer bindings above send `F12` + key, so the keys are the same inside and outside a project and never go over ssh. The mouse wheel and PgUp/PgDn reach the inner pane. A plain `split-window` or `new-window` in a wrapper acts on the outer wrapper, outside the project session.

## Behavior Inside/Outside Tmux

- In tmux, `finter` switches client to the selected session.
- If switch fails, it falls back to `tmux attach -t <session>`.
- If the selected project has no session, `finter` creates its wrapper first (local projects: `X` on the default server, attached to the inner session).

## Per-Project Session Workflow

Use one root per project collection (for example `projects`, `clients`, `experiments`) and keep one tmux session per folder.
The popup picker keeps this fast across many repositories.
