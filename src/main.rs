use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    author = "Xi Xiao",
    version = env!("CARGO_PKG_VERSION"),
    about = "A Tmux plugin to quickly create session for folders in configured paths."
)]
pub struct Args {
    #[arg(short('d'), long("directory"))]
    directories: Option<Vec<String>>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Record agent state in the current tmux pane (reads hook JSON on stdin)
    Hook {
        /// Agent that sends the hook; only `claude` is supported
        agent: String,
    },
    /// Print the tmux status-bar segment with waiting agents
    Status {
        /// Session the client is viewing (use `#{session_name}` in tmux); it is never badged
        #[arg(long)]
        current: Option<String>,
    },
    /// Jump to the oldest asking agent, then the oldest done one
    Next,
    /// Clear `done` badges for the current and previous session
    Seen {
        /// Also clear the session the client came from (for `client-session-changed`)
        #[arg(long)]
        leave: bool,
    },
    /// Show recent output of a session (used by the picker preview)
    Preview { target: String },
    /// Enter copy mode in the remote tmux behind the current pane
    CopyMode,
    /// Kill the current session; for a remote project also its sessions on the host
    Kill,
    /// Open a window; in a remote project it is a new terminal on the host
    NewWindow,
    /// Split the current pane side by side; in a wrapper the new pane is another terminal of the project
    SplitWindow {
        /// Stack the new pane below instead
        #[arg(long)]
        vertical: bool,
    },
    /// Print this machine's projects and agent panes (inner tmux server)
    List {
        #[arg(long)]
        json: bool,
    },
    /// Attach to (or create) the inner tmux session for a project; run by wrapper windows, locally or over ssh
    Attach {
        name: String,
        /// Use the terminal session `<name>~term`; `--term=N` (N >= 2) uses `<name>~termN`
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "1",
              value_parser = clap::value_parser!(u32).range(1..))]
        term: Option<u32>,
    },
}

// --------------------------------------------------
fn main() {
    let args = Args::parse();

    if let Err(e) = run(args) {
        eprint!("{}", e);
        std::process::exit(1);
    }
}

fn run(args: Args) -> finter::MyResult<()> {
    match (args.command, args.directories) {
        (Some(Command::Hook { agent }), _) => {
            // Hooks never fail: a non-zero exit could block the agent.
            finter::run_hook(&agent);
            Ok(())
        }
        (Some(Command::Status { current }), _) => {
            finter::run_status(current.as_deref());
            Ok(())
        }
        (Some(Command::Next), _) => {
            finter::run_next();
            Ok(())
        }
        (Some(Command::Seen { leave }), _) => {
            finter::run_seen(leave);
            Ok(())
        }
        (Some(Command::Preview { target }), _) => {
            finter::run_preview(&target);
            Ok(())
        }
        (Some(Command::CopyMode), _) => {
            finter::run_copy_mode();
            Ok(())
        }
        (Some(Command::Kill), _) => {
            finter::run_kill();
            Ok(())
        }
        (Some(Command::NewWindow), _) => {
            finter::run_new_window();
            Ok(())
        }
        (Some(Command::SplitWindow { vertical }), _) => {
            finter::run_split_window(vertical);
            Ok(())
        }
        (Some(Command::List { json }), _) => finter::run_list(json),
        (Some(Command::Attach { name, term }), _) => finter::run_attach(&name, term),
        (None, None) => finter::run_finter(),
        (None, Some(directories)) => finter::save_paths(&directories),
    }
}
