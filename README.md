# Finter

A Tmux session add-on that allows to quickly create and switch sessions based on
folders using fuzzy search.

[Github repo](https://github.com/xixiaofinland/finter)

# Prerequiste

- [Tmux](https://github.com/tmux/tmux)

# What does it do?

You give path(s) (e.g. `/home/username/projects/`) which includes folders
inside. `Finter` can easily create or jump between their Tmux sessions.


# Simple use case

Assume you have multiple folders in path `/home/username/projects/`:

1. run `finter /home/username/proejcts/` to tell finter where to search for
   folders
2. then run `finter`, it will pop up all folders in the pick list 
3. selecting by fuzzy search will send you to the Tmux session named by the
   folder name.

`Finter` allows you to easily have separated sessions for each folders.

I highly recommend you add a hotkey in `.tmux.config` (like mine below), so in Tmux you can quickly
call the popup windown to run `Finter`.

```
bind C-o display-popup -E "finter"  # `prefix-key C-o` will popup finter
selection list
```

![screenshot](https://github.com/xixiaofinland/finter/blob/master/pic/1.png)

# How to install

You need to have [Cargo command](https://www.rust-lang.org/tools/install) to install the tool
Run `cargo install finter` to install it locally.

# How it works?

For the first time, `finter` needs to know where your folders exists, 
so you need to config it:

- run `finter [absolute-path1] [absolute-path2] ...` to define one or multiple
   paths for this tool to search from. This command will save these paths in a
`.finter` file in your home directory

Note. it supports ONLY absolute path, like `/home/username/projects`, rather
than `~/projects`.

Once the path configration is done (you can verify the `~/.finter` file):

1. Run `finter` in terminal. It will list all folders in the defined paths
2. Select any folder in the popup will either spin up a new Tmux session and enter this
   folder, or enter back to the session if it exists already.

## To-Do?

- Display current session differently?
- Display existing sessions in the list differently?
- Build a method to kill the current session without existing tmux?
