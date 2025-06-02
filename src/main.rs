use clap::{Parser, Subcommand};

mod git;
use git::get_repo_manager;

mod docker;
use docker::get_docker_manager;

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
        #[arg(short, long, conflicts_with = "instructions")]
        description: Option<String>,

        /// Path to instructions file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        instructions: Option<String>,

        /// Specific agent to use (aider, claude-code)
        #[arg(short, long)]
        agent: Option<String>,

        /// Task timeout in minutes
        #[arg(long, default_value = "30")]
        timeout: u32,
    },
    /// Launch a Docker container for interactive debugging
    Debug {
        /// Unique identifier for the debug session
        #[arg(short, long)]
        name: String,
    },
    /// Stop the TSK proxy container
    StopProxy,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Quick {
            name,
            r#type,
            description,
            instructions,
            agent,
            timeout,
        } => {
            println!("Executing quick task: {}", name);
            println!("Type: {}", r#type);

            // Ensure either description or instructions is provided
            if description.is_none() && instructions.is_none() {
                eprintln!("Error: Either --description or --instructions must be provided");
                std::process::exit(1);
            }

            if let Some(ref desc) = description {
                println!("Description: {}", desc);
            }
            if let Some(ref inst) = instructions {
                println!("Instructions file: {}", inst);
            }
            if let Some(agent) = agent {
                println!("Agent: {}", agent);
            }
            println!("Timeout: {} minutes", timeout);

            // Copy repository for the task
            let repo_manager = get_repo_manager();
            match repo_manager.copy_repo(&name) {
                Ok((repo_path, branch_name)) => {
                    println!(
                        "Successfully created repository copy at: {}",
                        repo_path.display()
                    );

                    // Handle instructions file if provided
                    let mut instructions_file_path = None;
                    if let Some(ref inst_path) = instructions {
                        // Read the instructions file to verify it exists
                        match std::fs::read_to_string(inst_path) {
                            Ok(_) => {
                                // Copy instructions file to task folder (parent of repo_path)
                                let task_folder = repo_path.parent().unwrap();
                                let inst_filename = std::path::Path::new(inst_path)
                                    .file_name()
                                    .unwrap_or_else(|| std::ffi::OsStr::new("instructions.md"));
                                let dest_path = task_folder.join(inst_filename);

                                match std::fs::copy(inst_path, &dest_path) {
                                    Ok(_) => {
                                        println!(
                                            "Copied instructions file to: {}",
                                            dest_path.display()
                                        );
                                        instructions_file_path = Some(dest_path);
                                    }
                                    Err(e) => {
                                        eprintln!("Error copying instructions file: {}", e);
                                        std::process::exit(1);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error reading instructions file: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }

                    // Launch Docker container
                    match get_docker_manager() {
                        Ok(docker_manager) => {
                            println!("Launching Docker container...");

                            // Build the command based on whether we have instructions or description
                            let command = if let Some(ref inst_path) = instructions_file_path {
                                // Get just the filename for the container path
                                let inst_filename =
                                    inst_path.file_name().unwrap().to_str().unwrap();
                                vec![
                                    "sh".to_string(),
                                    "-c".to_string(),
                                    format!(
                                        "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions | jq",
                                        inst_filename
                                    )
                                    .to_string(),
                                ]
                            } else {
                                // Use description
                                let desc = description.as_ref().unwrap();
                                vec![
                                    "sh".to_string(),
                                    "-c".to_string(),
                                    format!(
                                        "claude -p --verbose --output-format stream-json --dangerously-skip-permissions '{}' | jq",
                                        desc
                                    )
                                    .to_string(),
                                ]
                            };

                            match docker_manager
                                .run_task_container(
                                    "tsk/base",
                                    &repo_path,
                                    command,
                                    instructions_file_path.as_ref(),
                                )
                                .await
                            {
                                Ok(output) => {
                                    println!("Container execution completed successfully");
                                    println!("Output:\n{}", output);

                                    // Commit any changes made by the container
                                    let commit_message =
                                        format!("TSK automated changes for task: {}", name);
                                    if let Err(e) =
                                        repo_manager.commit_changes(&repo_path, &commit_message)
                                    {
                                        eprintln!("Error committing changes: {}", e);
                                    }

                                    // Fetch changes back to main repository
                                    if let Err(e) =
                                        repo_manager.fetch_changes(&repo_path, &branch_name)
                                    {
                                        eprintln!("Error fetching changes: {}", e);
                                    } else {
                                        println!(
                                            "Branch {} is now available in the main repository",
                                            branch_name
                                        );
                                    }
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
                    eprintln!("Error copying repository: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Debug { name } => {
            println!("Starting debug session: {}", name);

            // Copy repository for the debug session
            let repo_manager = get_repo_manager();
            match repo_manager.copy_repo(&name) {
                Ok((repo_path, branch_name)) => {
                    println!(
                        "Successfully created repository copy at: {}",
                        repo_path.display()
                    );

                    // Launch Docker container
                    match get_docker_manager() {
                        Ok(docker_manager) => {
                            println!("Launching Docker container...");

                            match docker_manager
                                .create_debug_container("tsk/base", &repo_path)
                                .await
                            {
                                Ok(container_name) => {
                                    println!("\nDocker container started successfully!");
                                    println!("Container name: {}", container_name);
                                    println!("\nTo connect to the container, run:");
                                    println!("  docker exec -it {} /bin/bash", container_name);
                                    println!("\nPress any key to stop the container and exit...");

                                    // Wait for user input
                                    use std::io::{self, Read};
                                    let _ = io::stdin().read(&mut [0u8]).unwrap();

                                    println!("\nStopping container...");
                                    match docker_manager
                                        .stop_and_remove_container(&container_name)
                                        .await
                                    {
                                        Ok(_) => {
                                            println!("Container stopped and removed successfully");

                                            // Commit any changes made during debug session
                                            let commit_message =
                                                format!("TSK debug session changes for: {}", name);
                                            if let Err(e) = repo_manager
                                                .commit_changes(&repo_path, &commit_message)
                                            {
                                                eprintln!("Error committing changes: {}", e);
                                            }

                                            // Fetch changes back to main repository
                                            if let Err(e) =
                                                repo_manager.fetch_changes(&repo_path, &branch_name)
                                            {
                                                eprintln!("Error fetching changes: {}", e);
                                            } else {
                                                println!("Branch {} is now available in the main repository", branch_name);
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("Error stopping container: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Error creating debug container: {}", e);
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
                    eprintln!("Error copying repository: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::StopProxy => {
            println!("Stopping TSK proxy container...");
            match get_docker_manager() {
                Ok(docker_manager) => match docker_manager.stop_proxy().await {
                    Ok(_) => {
                        println!("Proxy container stopped successfully");
                    }
                    Err(e) => {
                        eprintln!("Error stopping proxy container: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error initializing Docker manager: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
