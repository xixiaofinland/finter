use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    author = "Xi Xiao",
    version = "0.1.4",
    about = "A Tmux plugin to quickly create session for folders in configured paths."
)]
pub struct Args {
    #[arg(short('d'), long("directory"))]
    directories: Option<Vec<String>>,
}

// --------------------------------------------------
fn main() {
    let args = Args::parse();
    println!("{:?}", args);
    if let Err(e) = run(args) {
        eprint!("{}", e);
        std::process::exit(1);
    }
}

fn run(args: Args) -> finter::MyResult<()> {
    match args.directories {
        None => finter::run_finter(),
        Some(directories) => finter::save_paths(&directories),
    }
}
