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

## Behavior Inside/Outside Tmux

- In tmux, `finter` switches client to the selected session.
- If switch fails, it falls back to `tmux attach -t <session>`.
- If selected session does not exist, `finter` creates it first.

## Per-Project Session Workflow

Use one root per project collection (for example `projects`, `clients`, `experiments`) and keep one tmux session per folder.
The popup picker keeps this fast across many repositories.
