# Usage

## CLI

`finter` has two modes:

- Configure roots: `finter -d <ABS_PATH> [ABS_PATH ...]`
- Run picker: `finter`

## Configure Project Roots

Set one or more absolute root paths where your project folders live:

```bash
finter -d /home/username/projects /home/username/work
```

This writes `~/.finter` and overwrites previous values.

`~/.finter` format:

```text
/home/username/projects
/home/username/work
```

Notes:

- Use absolute paths only.
- `~/...` is not expanded by `finter`.
- Invalid or non-existing paths are ignored at load time.

## Run Daily

Run `finter` from a shell or tmux popup:

```bash
finter
```

Flow:

1. Reads roots from `~/.finter`.
2. Finds immediate child directories under each root.
3. Reads current tmux sessions.
4. Shows combined list in fuzzy picker.
5. Select item to switch/create and attach.

## Typical Tmux Workflow

1. Configure roots once with `-d`.
2. Bind a tmux key to `display-popup -E "finter"`.
3. Hit hotkey, select folder, continue work in that session.
