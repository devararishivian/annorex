use clap::Parser;

/// Annotate the output of a coding agent in Rex and send the notes back.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {}

fn main() {
    let _args = Args::parse();
}
