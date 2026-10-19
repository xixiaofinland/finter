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
        (None, None) => finter::run_finter(),
        (None, Some(directories)) => finter::save_paths(&directories),
    }
}
