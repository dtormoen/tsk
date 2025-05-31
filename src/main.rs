use clap::Parser;

#[derive(Parser)]
#[command(name = "tsk")]
#[command(author, version, about = "TSK - Task delegation to AI agents", long_about = None)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}