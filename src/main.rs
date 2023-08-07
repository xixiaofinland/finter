// finter
// finter config --path (-p) multiple path

use clap::Parser;
use std::fs;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    paths: Vec<String>,
}

const PATH: &str = "/home/finxxi/projects";

fn main() {
    let cli = Cli::parse();

    let path_size = cli.paths.len();

    match path_size {
        0 => pop_up(),
        _ => println!("+++"),
    }

    // println!("name: {:?}", cli.config);
}

fn pop_up() {
    println!("{PATH}");

    let paths = fs::read_dir(PATH).unwrap();

    for path in paths {
        let path = path.unwrap().path();
        if path.is_dir() {
            println!("Name: {}", path.display());
        }
    }
}
