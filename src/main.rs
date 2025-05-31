use clap::{Parser, Subcommand};

mod git;
use git::WorktreeManager;

mod docker;
use docker::DockerManager;

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

#[tokio::main]
async fn main() {
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

                    // Launch Docker container
                    match DockerManager::new() {
                        Ok(docker_manager) => {
                            println!("Launching Docker container...");

                            // Command to create/append to tsk-test.md
                            let timestamp = chrono::Utc::now().to_rfc3339();
                            let log_entry = format!(
                                "[{}] Task: {} | Type: {} | Description: {}",
                                timestamp, name, r#type, description
                            );

                            // Debug: List directory contents before and after
                            let command = vec![
                                "sh".to_string(),
                                "-c".to_string(),
                                format!(
                                    "pwd && ls -la && touch tsk-test.md && echo '{}' >> tsk-test.md && echo 'File created:' && ls -la tsk-test.md && cat tsk-test.md",
                                    log_entry
                                ),
                            ];

                            match docker_manager
                                .run_task_container("tsk/base", &worktree_path, command)
                                .await
                            {
                                Ok(output) => {
                                    println!("Container execution completed successfully");
                                    println!("Output:\n{}", output);
                                }
                                Err(e) => {
                                    eprintln!("Error running container: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error initializing Docker manager: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error creating worktree: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
