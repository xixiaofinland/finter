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
    Status,
    /// Jump to the oldest asking agent, then the oldest done one
    Next,
    /// Clear `done` badges for the current and previous session
    Seen,
    /// Show recent output of a session (used by the picker preview)
    Preview { target: String },
    /// Enter copy mode in the remote tmux behind the current pane
    CopyMode,
    /// Print this machine's projects and agent panes (inner tmux server)
    List {
        #[arg(long)]
        json: bool,
    },
    /// Attach to (or create) the inner tmux session for a project; run on the remote host
    Attach {
        name: String,
        /// Use the terminal session `<name>~term`
        #[arg(long)]
        term: bool,
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
        (Some(Command::Status), _) => {
            finter::run_status();
            Ok(())
        }
        (Some(Command::Next), _) => {
            finter::run_next();
            Ok(())
        }
        (Some(Command::Seen), _) => {
            finter::run_seen();
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
        (Some(Command::List { json }), _) => finter::run_list(json),
        (Some(Command::Attach { name, term }), _) => finter::run_attach(&name, term),
        (None, None) => finter::run_finter(),
        (None, Some(directories)) => finter::save_paths(&directories),
    }
}
