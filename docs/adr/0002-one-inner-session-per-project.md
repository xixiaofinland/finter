# One inner session per project, wrappers are one pane

Since 0.4 every terminal of a project is its own inner session (`X`, `X~term`, `X~termN`) and each machine's wrapper holds one window per terminal, so a window opened on one machine never shows up in another machine's wrapper of the same project. We decided that a project is a single inner session `X` whose windows and panes are the project's terminals, and a wrapper (local `X` or remote `host/X`) is one window with one pane attached to it. Every machine then shows the same windows, and opening, splitting or closing them is plain tmux inside the inner session instead of finter keeping wrappers in step.

## Considered Options

- **Keep one inner session per terminal and reconcile wrappers** (add a window for every `X~termN` when creating or entering a wrapper). Rejected: wrappers can still drift between reconciliations, splits cannot be reproduced, and each new tmux action needs its own forwarding command (`new-window`, `split-window`, `copy-mode`).
- **Grouped inner sessions per machine** (same windows, independent current window). Rejected for now: plain shared attach is simpler, and following the other machine's current window does not matter in practice.
- **Draw the inner window list in the outer status bar.** Rejected: it needs polling (ssh or the 4s cache for remote projects) and lags behind; the inner status bar is immediate.

## Consequences

- All clients of a project share its current window: switching windows on one machine switches it on the others.
- The inner server turns its status bar on to show the window list, so a project shows two bars: the outer one (session, agent badges) and the inner one (windows). The outer window list hides in wrappers.
- Project-level keys reach the inner server through the terminal, not over ssh: outer bindings for `n`, `p`, new window, split, `q`, `[` and `1`-`9` check `@finter_remote` and send `F12` plus the key, which the inner server binds in its own key table. `F12` is never typed on its own, unlike the outer prefix `C-a`, which the shell inside still needs. Session-level keys (`f`, `a`, `u`, `x`) stay on the outer server.
- vim-tmux-navigator (`C-h/j/k/l`) is not forwarded: in a wrapper it sees ssh or finter, not vim, and the outer window has a single pane. Moving between panes inside a project uses the mouse until this is needed.
- `next`, `seen`, `kill` and the picker preview address inner windows and panes of `X` instead of `X~termN` sessions; `finter new-window`, `finter split-window` and `finter copy-mode` are removed.
- No migration: on upgrade each machine runs `tmux -L finter kill-server` once and reopens its projects (running agents are resumed with `claude --resume`).
