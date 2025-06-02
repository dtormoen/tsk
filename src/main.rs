use clap::{Parser, Subcommand};

mod git;
use git::get_repo_manager;

mod docker;
use docker::get_docker_manager;

mod task;
use task::{get_task_storage, Task, TaskStatus};

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

            // Copy instructions file to task directory if provided
            let instructions_path = if let Some(ref inst_path) = instructions {
                // Verify instructions file exists
                match std::fs::read_to_string(inst_path) {
                    Ok(_) => {
                        // Create task directory
                        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
                        let task_dir_name = format!("{}-{}", timestamp, name);
                        let task_dir = std::path::Path::new(".tsk/tasks").join(&task_dir_name);
                        std::fs::create_dir_all(&task_dir).unwrap_or_else(|e| {
                            eprintln!("Error creating task directory: {}", e);
                            std::process::exit(1);
                        });

                        // Copy instructions file to task directory
                        let inst_filename = std::path::Path::new(inst_path)
                            .file_name()
                            .unwrap_or_else(|| std::ffi::OsStr::new("instructions.md"));
                        let dest_path = task_dir.join(inst_filename);

                        match std::fs::copy(inst_path, &dest_path) {
                            Ok(_) => {
                                println!("Copied instructions file to task directory");
                                Some(dest_path.to_string_lossy().to_string())
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
            } else {
                None
            };

            // Create and save the task
            let task = Task::new(
                name.clone(),
                r#type.clone(),
                description.clone(),
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

                            for task in queued_tasks {
                                println!("\n{}", "=".repeat(60));
                                println!("Running task: {} ({})", task.name, task.id);
                                println!("Type: {}", task.task_type);
                                if let Some(ref desc) = task.description {
                                    println!("Description: {}", desc);
                                }
                                println!("{}", "=".repeat(60));

                                // Update task status to running
                                let mut running_task = task.clone();
                                running_task.status = TaskStatus::Running;
                                running_task.started_at = Some(chrono::Utc::now());

                                if let Err(e) = storage.update_task(running_task.clone()).await {
                                    eprintln!("Error updating task status: {}", e);
                                    continue;
                                }

                                // Copy repository for the task
                                let repo_manager = get_repo_manager();
                                match repo_manager.copy_repo(&task.name) {
                                    Ok((repo_path, branch_name)) => {
                                        println!(
                                            "Created repository copy at: {}",
                                            repo_path.display()
                                        );

                                        // Handle instructions file if provided
                                        let mut instructions_file_path = None;
                                        if let Some(ref inst_path) = task.instructions_file {
                                            match std::fs::read_to_string(inst_path) {
                                                Ok(_) => {
                                                    let task_folder = repo_path.parent().unwrap();
                                                    let inst_filename =
                                                        std::path::Path::new(inst_path)
                                                            .file_name()
                                                            .unwrap_or_else(|| {
                                                                std::ffi::OsStr::new(
                                                                    "instructions.md",
                                                                )
                                                            });
                                                    let dest_path = task_folder.join(inst_filename);

                                                    match std::fs::copy(inst_path, &dest_path) {
                                                        Ok(_) => {
                                                            instructions_file_path =
                                                                Some(dest_path);
                                                        }
                                                        Err(e) => {
                                                            eprintln!("Error copying instructions file: {}", e);
                                                            let mut failed_task =
                                                                running_task.clone();
                                                            failed_task.status = TaskStatus::Failed;
                                                            failed_task.error_message = Some(format!("Failed to copy instructions: {}", e));
                                                            failed_task.completed_at =
                                                                Some(chrono::Utc::now());
                                                            let _ = storage
                                                                .update_task(failed_task)
                                                                .await;
                                                            continue;
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "Error reading instructions file: {}",
                                                        e
                                                    );
                                                    let mut failed_task = running_task.clone();
                                                    failed_task.status = TaskStatus::Failed;
                                                    failed_task.error_message = Some(format!(
                                                        "Failed to read instructions: {}",
                                                        e
                                                    ));
                                                    failed_task.completed_at =
                                                        Some(chrono::Utc::now());
                                                    let _ = storage.update_task(failed_task).await;
                                                    continue;
                                                }
                                            }
                                        }

                                        // Launch Docker container
                                        match get_docker_manager() {
                                            Ok(docker_manager) => {
                                                println!("Launching Docker container...");

                                                // Build the command
                                                let command = if let Some(ref inst_path) =
                                                    instructions_file_path
                                                {
                                                    let inst_filename = inst_path
                                                        .file_name()
                                                        .unwrap()
                                                        .to_str()
                                                        .unwrap();
                                                    vec![
                                                        "sh".to_string(),
                                                        "-c".to_string(),
                                                        format!(
                                                            "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions | jq",
                                                            inst_filename
                                                        ),
                                                    ]
                                                } else if let Some(ref desc) = task.description {
                                                    vec![
                                                        "sh".to_string(),
                                                        "-c".to_string(),
                                                        format!(
                                                            "claude -p --verbose --output-format stream-json --dangerously-skip-permissions '{}' | jq",
                                                            desc
                                                        ),
                                                    ]
                                                } else {
                                                    eprintln!("Task has neither description nor instructions");
                                                    let mut failed_task = running_task.clone();
                                                    failed_task.status = TaskStatus::Failed;
                                                    failed_task.error_message = Some(
                                                        "No description or instructions provided"
                                                            .to_string(),
                                                    );
                                                    failed_task.completed_at =
                                                        Some(chrono::Utc::now());
                                                    let _ = storage.update_task(failed_task).await;
                                                    continue;
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
                                                        println!("\nTask completed successfully");
                                                        println!("Output:\n{}", output);

                                                        // Commit any changes
                                                        let commit_message = format!(
                                                            "TSK automated changes for task: {}",
                                                            task.name
                                                        );
                                                        if let Err(e) = repo_manager.commit_changes(
                                                            &repo_path,
                                                            &commit_message,
                                                        ) {
                                                            eprintln!(
                                                                "Error committing changes: {}",
                                                                e
                                                            );
                                                        }

                                                        // Fetch changes back
                                                        if let Err(e) = repo_manager
                                                            .fetch_changes(&repo_path, &branch_name)
                                                        {
                                                            eprintln!(
                                                                "Error fetching changes: {}",
                                                                e
                                                            );
                                                        } else {
                                                            println!("Branch {} is now available in the main repository", branch_name);
                                                        }

                                                        // Update task status to complete
                                                        let mut complete_task =
                                                            running_task.clone();
                                                        complete_task.status = TaskStatus::Complete;
                                                        complete_task.completed_at =
                                                            Some(chrono::Utc::now());
                                                        complete_task.branch_name =
                                                            Some(branch_name);
                                                        let _ = storage
                                                            .update_task(complete_task)
                                                            .await;
                                                    }
                                                    Err(e) => {
                                                        eprintln!("Error running container: {}", e);
                                                        let mut failed_task = running_task.clone();
                                                        failed_task.status = TaskStatus::Failed;
                                                        failed_task.error_message = Some(format!(
                                                            "Container execution failed: {}",
                                                            e
                                                        ));
                                                        failed_task.completed_at =
                                                            Some(chrono::Utc::now());
                                                        let _ =
                                                            storage.update_task(failed_task).await;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "Error initializing Docker manager: {}",
                                                    e
                                                );
                                                let mut failed_task = running_task.clone();
                                                failed_task.status = TaskStatus::Failed;
                                                failed_task.error_message = Some(format!(
                                                    "Docker initialization failed: {}",
                                                    e
                                                ));
                                                failed_task.completed_at = Some(chrono::Utc::now());
                                                let _ = storage.update_task(failed_task).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error copying repository: {}", e);
                                        let mut failed_task = running_task.clone();
                                        failed_task.status = TaskStatus::Failed;
                                        failed_task.error_message =
                                            Some(format!("Repository copy failed: {}", e));
                                        failed_task.completed_at = Some(chrono::Utc::now());
                                        let _ = storage.update_task(failed_task).await;
                                    }
                                }
                            }

                            println!("\n{}", "=".repeat(60));
                            println!("All tasks processed!");
                            println!("Use 'tsk list' to see the final status of all tasks");
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
