# The Need for Finter

I use Tmux to handle sessions and windows in terminal. [my Tmux config
file](https://github.com/xixiaofinland/dotfiles/blob/main/.tmux.conf).

Everything is cool, except one feature is missing... What is it?

I want to quickly list a bunch of folders and create a Tmux session with
the folder name I select.

This helps me to mentally isolate the work between folders/projects. 

For instance, I hit a hotkey to pop up the list of folders/projects, I select one by fuzzy search, bang! the new session is created.

Finter is born to fulfil this demand.

[Github repo](https://github.com/xixiaofinland/finter)

# How it works

1. Define the path(s) where folders reside, e.g. `finter /home/username/proejcts/`
2. Add `Finter` hotkey in the `.tmux.conf` so it opens in a pop up (config
   below)
3. Run the hotkey `Prefix C-o` in Tmux to create or jump to that Tmux session
   (screenshot below)

```
bind C-o display-popup -E "finter"  # `prefix-key C-o` will popup finter
selection list
```

![screenshot](https://github.com/xixiaofinland/finter/blob/master/pic/1.png)

# How to install

You need to have [Cargo command](https://www.rust-lang.org/tools/install) to install the tool
Run `cargo install finter` to install it locally.

# How it works?

Check the source code or short description below.

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
