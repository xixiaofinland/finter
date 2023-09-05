# Finter

A simple Cli to use Tmux to manage my projects

[Github repo](https://github.com/xixiaofinland/finter)

# Prerequiste

- [Tmux](https://github.com/tmux/tmux)
- [Cargo command](https://www.rust-lang.org/tools/install) to install the tool

# What does it do?

Assume you have a path (e.g. `/home/username/projects/`) containing multiple
project folders inside. You want to spin up dedicated Tmux session for each
project and can easily jump/switch between these sessions.

Then this Cli tool is for you!

# How to install

Run `cargo install finter` to install it locally.

# Simple use case

Assume you have multiple projects in path `/home/username/projects/`:

1. run `finter /home/username/proejcts/`
2. run `finter`, it will pop up all folders in your project path to select
3. select one will send you to the Tmux session named by your project

I highly recommend you add a hotkey in `.tmux.config` (like mine below), so in Tmux you can quickly
call the popup windown to create or switch Tmux sessions.

```
bind C-o display-popup -E "finter"  # `prefix-key C-o` will popup finter
selection list
```
# How it works?

For the first time, `finter` needs to know where your project folders exists, 
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
