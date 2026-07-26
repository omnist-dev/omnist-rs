use clap::Parser;
use omnist_cli::{Cli, run};

fn main() {
    let cli = Cli::parse();
    std::process::exit(run(cli));
}
