use clap::Parser;

#[derive(Debug, Parser)]
#[command(author, version, about)]
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
