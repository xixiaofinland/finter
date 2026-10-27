# Project sessions live on the inner server

Since 0.4, a machine's projects opened from another host run on its `tmux -L finter` server, while the same projects opened locally run on the default server, so a machine used both directly and as a host ends up with two separate sessions per project. We decided that every project session lives on the inner server of the machine that owns the project, and the default server only holds wrappers: a local wrapper attaches to the inner session directly, a remote wrapper over ssh. One project then has exactly one session wherever it is opened from, with no per-machine configuration.

## Considered Options

- **Attach remote wrappers to the host's default server (the 0.3 model).** Rejected: prefix and status bar are server/session settings, not per-client, so one session cannot be the keyed outer tmux at the machine and the keyless inner one inside another host's tmux. Remote use would mean two status bars and `C-a C-a`.
- **A per-machine "local host" entry that lists the machine's own inner server.** Rejected: both sessions keep existing, and every machine that becomes a host needs the entry, or the split quietly comes back.

## Consequences

- Local projects are nested too. The inner layer has no prefix and no status bar, so the outer tmux behaves as before.
- `split-window` in a wrapper splits the outer wrapper pane: the new shell is outside the project's session and starts in the wrapper's cwd. Accepted for now (splits are rarely used); routing splits to the inner server is a later change if needed. Since 0.6, `finter split-window` splits the wrapper pane with another project terminal (`X~termN`), the same way `finter new-window` adds windows.
- Two clients of different sizes on one inner session resize it to the latest active one.
- The inner server keeps the environment of whichever `finter attach` started it, local or over ssh.
