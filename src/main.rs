use clap::{Parser, Subcommand};

mod git;

mod docker;
use docker::get_docker_manager;

mod task;
use task::{get_task_storage, Task, TaskStatus};

mod task_manager;
use task_manager::TaskManager;

mod log_processor;

use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(name = "tsk")]
#[command(author, version, about = "TSK - Task delegation to AI agents", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Queue a task for later execution
    Add {
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
    /// List all queued tasks
    List,
    /// Run all queued tasks sequentially
    Run,
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
        Commands::Add {
            name,
            r#type,
            description,
            instructions,
            agent,
            timeout,
        } => {
            println!("Adding task to queue: {}", name);

            // Ensure either description or instructions is provided
            if description.is_none() && instructions.is_none() {
                eprintln!("Error: Either --description or --instructions must be provided");
                std::process::exit(1);
            }

            // Validate task type
            let valid_types = [
                "code-review",
                "refactor",
                "feature",
                "bug-fix",
                "test-generation",
                "documentation",
            ];
            if !valid_types.contains(&r#type.as_str()) {
                eprintln!(
                    "Error: Invalid task type '{}'. Valid types are: {}",
                    r#type,
                    valid_types.join(", ")
                );
                std::process::exit(1);
            }

            // Create task directory
            let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
            let task_dir_name = format!("{}-{}", timestamp, name);
            let task_dir = std::path::Path::new(".tsk/tasks").join(&task_dir_name);
            std::fs::create_dir_all(&task_dir).unwrap_or_else(|e| {
                eprintln!("Error creating task directory: {}", e);
                std::process::exit(1);
            });

            // Always create an instructions.md file
            let instructions_path = if let Some(ref inst_path) = instructions {
                // Copy existing instructions file
                match std::fs::read_to_string(inst_path) {
                    Ok(content) => {
                        let dest_path = task_dir.join("instructions.md");
                        match std::fs::write(&dest_path, content) {
                            Ok(_) => {
                                println!("Copied instructions file to task directory");
                                Some(dest_path.to_string_lossy().to_string())
                            }
                            Err(e) => {
                                eprintln!("Error writing instructions file: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading instructions file: {}", e);
                        std::process::exit(1);
                    }
                }
            } else if let Some(ref desc) = description {
                // Check if a template exists for this task type
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                let content = if template_path.exists() {
                    // Load and process template
                    match std::fs::read_to_string(&template_path) {
                        Ok(template_content) => {
                            // Simple template processing - replace {{DESCRIPTION}} with the actual description
                            template_content.replace("{{DESCRIPTION}}", desc)
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to read template file: {}", e);
                            // Fall back to plain description
                            desc.clone()
                        }
                    }
                } else {
                    // No template found, use plain description
                    desc.clone()
                };

                // Create instructions.md with processed content
                let dest_path = task_dir.join("instructions.md");
                match std::fs::write(&dest_path, content) {
                    Ok(_) => {
                        if template_path.exists() {
                            println!("Created instructions file from {} template", r#type);
                        } else {
                            println!("Created instructions file from description");
                        }
                        Some(dest_path.to_string_lossy().to_string())
                    }
                    Err(e) => {
                        eprintln!("Error creating instructions file: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // This should never happen due to validation, but handle it gracefully
                eprintln!("Error: No description or instructions provided");
                std::process::exit(1);
            };

            // Create and save the task
            let task = Task::new(
                name.clone(),
                r#type.clone(),
                None, // description is now stored in instructions file
                instructions_path,
                agent.clone(),
                timeout,
            );

            match get_task_storage() {
                Ok(storage) => match storage.add_task(task.clone()).await {
                    Ok(_) => {
                        println!("\nTask successfully added to queue!");
                        println!("Task ID: {}", task.id);
                        println!("Type: {}", r#type);
                        if let Some(desc) = description {
                            println!("Description: {}", desc);
                        }
                        if instructions.is_some() {
                            println!("Instructions: Copied to task directory");
                        }
                        if let Some(agent) = agent {
                            println!("Agent: {}", agent);
                        }
                        println!("Timeout: {} minutes", timeout);
                        println!("\nUse 'tsk list' to view all queued tasks");
                        println!("Use 'tsk run' to execute the next task in the queue");
                    }
                    Err(e) => {
                        eprintln!("Error saving task: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error initializing task storage: {}", e);
                    std::process::exit(1);
                }
            }
        }
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

            // Create task directory for quick tasks
            let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
            let task_dir_name = format!("{}-{}", timestamp, name);
            let task_dir = std::path::Path::new(".tsk/quick-tasks").join(&task_dir_name);
            std::fs::create_dir_all(&task_dir).unwrap_or_else(|e| {
                eprintln!("Error creating task directory: {}", e);
                std::process::exit(1);
            });

            // Always create an instructions.md file
            let instructions_path = if let Some(ref inst_path) = instructions {
                // Copy existing instructions file
                match std::fs::read_to_string(inst_path) {
                    Ok(content) => {
                        let dest_path = task_dir.join("instructions.md");
                        match std::fs::write(&dest_path, content) {
                            Ok(_) => {
                                println!("Copied instructions file to task directory");
                                dest_path.to_string_lossy().to_string()
                            }
                            Err(e) => {
                                eprintln!("Error writing instructions file: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading instructions file: {}", e);
                        std::process::exit(1);
                    }
                }
            } else if let Some(ref desc) = description {
                // Check if a template exists for this task type
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                let content = if template_path.exists() {
                    // Load and process template
                    match std::fs::read_to_string(&template_path) {
                        Ok(template_content) => {
                            // Simple template processing - replace {{DESCRIPTION}} with the actual description
                            template_content.replace("{{DESCRIPTION}}", desc)
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to read template file: {}", e);
                            // Fall back to plain description
                            desc.clone()
                        }
                    }
                } else {
                    // No template found, use plain description
                    desc.clone()
                };

                // Create instructions.md with processed content
                let dest_path = task_dir.join("instructions.md");
                match std::fs::write(&dest_path, content) {
                    Ok(_) => {
                        if template_path.exists() {
                            println!("Created instructions file from {} template", r#type);
                        } else {
                            println!("Created instructions file from description");
                        }
                        dest_path.to_string_lossy().to_string()
                    }
                    Err(e) => {
                        eprintln!("Error creating instructions file: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // This should never happen due to validation
                eprintln!("Error: No description or instructions provided");
                std::process::exit(1);
            };

            if let Some(agent) = agent {
                println!("Agent: {}", agent);
            }
            println!("Timeout: {} minutes", timeout);

            // Use TaskManager to execute the task
            match TaskManager::new() {
                Ok(task_manager) => {
                    match task_manager
                        .execute_task(&name, None, Some(&instructions_path))
                        .await
                    {
                        Ok(_) => {
                            // Task completed successfully
                        }
                        Err(e) => {
                            eprintln!("{}", e.message);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error initializing task manager: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Debug { name } => {
            println!("Starting debug session: {}", name);

            // Use TaskManager to run the debug container
            match TaskManager::new() {
                Ok(task_manager) => {
                    if let Err(e) = task_manager.run_debug_container(&name).await {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error initializing task manager: {}", e);
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
        Commands::List => {
            match get_task_storage() {
                Ok(storage) => match storage.list_tasks().await {
                    Ok(tasks) => {
                        if tasks.is_empty() {
                            println!("No tasks in queue");
                        } else {
                            // Create table data structure
                            #[derive(Tabled)]
                            struct TaskRow {
                                #[tabled(rename = "ID")]
                                id: String,
                                #[tabled(rename = "Name")]
                                name: String,
                                #[tabled(rename = "Type")]
                                task_type: String,
                                #[tabled(rename = "Status")]
                                status: String,
                                #[tabled(rename = "Agent")]
                                agent: String,
                                #[tabled(rename = "Created")]
                                created: String,
                            }

                            let rows: Vec<TaskRow> = tasks
                                .iter()
                                .map(|task| TaskRow {
                                    id: task.id.clone(),
                                    name: task.name.clone(),
                                    task_type: task.task_type.clone(),
                                    status: match &task.status {
                                        TaskStatus::Queued => "QUEUED".to_string(),
                                        TaskStatus::Running => "RUNNING".to_string(),
                                        TaskStatus::Failed => "FAILED".to_string(),
                                        TaskStatus::Complete => "COMPLETE".to_string(),
                                    },
                                    agent: task.agent.clone().unwrap_or_else(|| "auto".to_string()),
                                    created: task.created_at.format("%Y-%m-%d %H:%M").to_string(),
                                })
                                .collect();

                            let table = Table::new(rows).with(Style::modern()).to_string();
                            println!("{}", table);

                            // Print summary
                            let queued = tasks
                                .iter()
                                .filter(|t| t.status == TaskStatus::Queued)
                                .count();
                            let running = tasks
                                .iter()
                                .filter(|t| t.status == TaskStatus::Running)
                                .count();
                            let complete = tasks
                                .iter()
                                .filter(|t| t.status == TaskStatus::Complete)
                                .count();
                            let failed = tasks
                                .iter()
                                .filter(|t| t.status == TaskStatus::Failed)
                                .count();

                            println!(
                                "\nSummary: {} queued, {} running, {} complete, {} failed",
                                queued, running, complete, failed
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("Error listing tasks: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error initializing task storage: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Run => {
            match get_task_storage() {
                Ok(storage) => {
                    // Get all queued tasks
                    match storage.list_tasks().await {
                        Ok(tasks) => {
                            let queued_tasks: Vec<Task> = tasks
                                .into_iter()
                                .filter(|t| t.status == TaskStatus::Queued)
                                .collect();

                            if queued_tasks.is_empty() {
                                println!("No queued tasks to run");
                                return;
                            }

                            println!("Found {} queued task(s) to run", queued_tasks.len());

                            // Use TaskManager with storage to execute tasks
                            match TaskManager::with_storage() {
                                Ok(task_manager) => {
                                    for task in queued_tasks {
                                        println!("\n{}", "=".repeat(60));
                                        println!("Running task: {} ({})", task.name, task.id);
                                        println!("Type: {}", task.task_type);
                                        if let Some(ref desc) = task.description {
                                            println!("Description: {}", desc);
                                        }
                                        println!("{}", "=".repeat(60));

                                        // Execute the task with automatic status updates
                                        match task_manager.execute_queued_task(&task).await {
                                            Ok(_result) => {
                                                println!("\nTask completed successfully");
                                                // The task manager handles all status updates
                                            }
                                            Err(e) => {
                                                eprintln!("Task failed: {}", e.message);
                                                // The task manager handles status updates
                                            }
                                        }
                                    }

                                    println!("\n{}", "=".repeat(60));
                                    println!("All tasks processed!");
                                    println!("Use 'tsk list' to see the final status of all tasks");
                                }
                                Err(e) => {
                                    eprintln!("Error initializing task manager: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error listing tasks: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error initializing task storage: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
