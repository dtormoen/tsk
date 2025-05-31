use clap::{Parser, Subcommand};

mod git;
use git::WorktreeManager;

#[derive(Parser)]
#[command(name = "tsk")]
#[command(author, version, about = "TSK - Task delegation to AI agents", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Immediately execute a task without queuing
    Quick {
        /// Unique identifier for the task
        #[arg(short, long)]
        name: String,

        /// Task type (code-review, refactor, feature, bug-fix, test-generation, documentation)
        #[arg(short = 't', long)]
        r#type: String,

        /// Detailed description of what needs to be accomplished
        #[arg(short, long)]
        description: String,

        /// Specific agent to use (aider, claude-code)
        #[arg(short, long)]
        agent: Option<String>,

        /// Task timeout in minutes
        #[arg(long, default_value = "30")]
        timeout: u32,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Quick {
            name,
            r#type,
            description,
            agent,
            timeout,
        } => {
            println!("Executing quick task: {}", name);
            println!("Type: {}", r#type);
            println!("Description: {}", description);
            if let Some(agent) = agent {
                println!("Agent: {}", agent);
            }
            println!("Timeout: {} minutes", timeout);

            // Create worktree for the task
            let worktree_manager = WorktreeManager::new();
            match worktree_manager.create_worktree(&name) {
                Ok(worktree_path) => {
                    println!(
                        "Successfully created worktree at: {}",
                        worktree_path.display()
                    );
                    // TODO: Execute agent in the worktree
                }
                Err(e) => {
                    eprintln!("Error creating worktree: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
