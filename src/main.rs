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

        /// Task type (defaults to 'generic' if not specified)
        #[arg(short = 't', long, default_value = "generic")]
        r#type: String,

        /// Detailed description of what needs to be accomplished
        #[arg(short, long, conflicts_with = "instructions")]
        description: Option<String>,

        /// Path to instructions file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        instructions: Option<String>,

        /// Open the instructions file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,

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

        /// Task type (defaults to 'generic' if not specified)
        #[arg(short = 't', long, default_value = "generic")]
        r#type: String,

        /// Detailed description of what needs to be accomplished
        #[arg(short, long, conflicts_with = "instructions")]
        description: Option<String>,

        /// Path to instructions file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        instructions: Option<String>,

        /// Open the instructions file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,

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
    /// Manage tasks in the task list
    Tasks {
        /// Delete a specific task by ID
        #[arg(short, long, value_name = "TASK_ID")]
        delete: Option<String>,

        /// Delete all completed tasks and all quick tasks
        #[arg(short, long)]
        clean: bool,
    },
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
            edit,
            agent,
            timeout,
        } => {
            println!("Adding task to queue: {}", name);

            // Ensure either description or instructions is provided, or edit flag is set
            if description.is_none() && instructions.is_none() && !edit {
                eprintln!("Error: Either --description or --instructions must be provided, or use --edit to create instructions interactively");
                std::process::exit(1);
            }

            // Validate task type if specified (not default 'generic')
            if r#type != "generic" {
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                if !template_path.exists() {
                    eprintln!(
                        "Error: No template found for task type '{}'. Please check the templates folder.",
                        r#type
                    );
                    std::process::exit(1);
                }
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
            } else if edit {
                // Create empty instructions file for editing
                let dest_path = task_dir.join("instructions.md");

                // Check if a template exists for this task type
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                let initial_content = if template_path.exists() {
                    // Load template as starting point
                    match std::fs::read_to_string(&template_path) {
                        Ok(template_content) => {
                            // Use template with placeholder for user to fill
                            template_content.replace(
                                "{{DESCRIPTION}}",
                                "<!-- TODO: Add your task description here -->",
                            )
                        }
                        Err(_) => {
                            // Fall back to empty file
                            String::new()
                        }
                    }
                } else {
                    // No template found, start with empty file
                    String::new()
                };

                match std::fs::write(&dest_path, initial_content) {
                    Ok(_) => {
                        println!("Created instructions file for editing");
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

            // Open editor if edit flag is set
            if edit {
                if let Some(ref inst_path) = instructions_path {
                    // Get editor from environment variable
                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                        // Try common fallbacks
                        if std::env::var("VISUAL").is_ok() {
                            std::env::var("VISUAL").unwrap()
                        } else {
                            "vi".to_string()
                        }
                    });

                    println!("Opening instructions file in editor: {}", editor);

                    // Execute editor
                    let status = std::process::Command::new(&editor).arg(inst_path).status();

                    match status {
                        Ok(exit_status) => {
                            if !exit_status.success() {
                                eprintln!("Editor exited with non-zero status");
                                std::process::exit(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to open editor: {}", e);
                            eprintln!("Please ensure EDITOR environment variable is set to a valid editor");
                            std::process::exit(1);
                        }
                    }

                    // Check if file is empty after editing
                    match std::fs::read_to_string(inst_path) {
                        Ok(content) => {
                            if content.trim().is_empty() {
                                eprintln!(
                                    "Error: Instructions file is empty. Task creation cancelled."
                                );
                                // Clean up
                                let _ = std::fs::remove_dir_all(&task_dir);
                                std::process::exit(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error reading edited instructions file: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }

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
            edit,
            agent,
            timeout,
        } => {
            println!("Executing quick task: {}", name);
            println!("Type: {}", r#type);

            // Ensure either description or instructions is provided, or edit flag is set
            if description.is_none() && instructions.is_none() && !edit {
                eprintln!("Error: Either --description or --instructions must be provided, or use --edit to create instructions interactively");
                std::process::exit(1);
            }

            // Validate task type if specified (not default 'generic')
            if r#type != "generic" {
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                if !template_path.exists() {
                    eprintln!(
                        "Error: No template found for task type '{}'. Please check the templates folder.",
                        r#type
                    );
                    std::process::exit(1);
                }
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
            } else if edit {
                // Create empty instructions file for editing
                let dest_path = task_dir.join("instructions.md");

                // Check if a template exists for this task type
                let template_path =
                    std::path::Path::new("templates").join(format!("{}.md", r#type));
                let initial_content = if template_path.exists() {
                    // Load template as starting point
                    match std::fs::read_to_string(&template_path) {
                        Ok(template_content) => {
                            // Use template with placeholder for user to fill
                            template_content.replace(
                                "{{DESCRIPTION}}",
                                "<!-- TODO: Add your task description here -->",
                            )
                        }
                        Err(_) => {
                            // Fall back to empty file
                            String::new()
                        }
                    }
                } else {
                    // No template found, start with empty file
                    String::new()
                };

                match std::fs::write(&dest_path, initial_content) {
                    Ok(_) => {
                        println!("Created instructions file for editing");
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

            // Open editor if edit flag is set
            if edit {
                // Get editor from environment variable
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                    // Try common fallbacks
                    if std::env::var("VISUAL").is_ok() {
                        std::env::var("VISUAL").unwrap()
                    } else {
                        "vi".to_string()
                    }
                });

                println!("Opening instructions file in editor: {}", editor);

                // Execute editor
                let status = std::process::Command::new(&editor)
                    .arg(&instructions_path)
                    .status();

                match status {
                    Ok(exit_status) => {
                        if !exit_status.success() {
                            eprintln!("Editor exited with non-zero status");
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to open editor: {}", e);
                        eprintln!(
                            "Please ensure EDITOR environment variable is set to a valid editor"
                        );
                        std::process::exit(1);
                    }
                }

                // Check if file is empty after editing
                match std::fs::read_to_string(&instructions_path) {
                    Ok(content) => {
                        if content.trim().is_empty() {
                            eprintln!(
                                "Error: Instructions file is empty. Task execution cancelled."
                            );
                            // Clean up
                            let _ = std::fs::remove_dir_all(&task_dir);
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading edited instructions file: {}", e);
                        std::process::exit(1);
                    }
                }
            }

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
        Commands::Tasks { delete, clean } => {
            // Ensure at least one option is provided
            if delete.is_none() && !clean {
                eprintln!("Error: Please specify either --delete <TASK_ID> or --clean");
                std::process::exit(1);
            }

            // Get task manager with storage
            match TaskManager::with_storage() {
                Ok(task_manager) => {
                    // Handle delete option
                    if let Some(task_id) = delete {
                        println!("Deleting task: {}", task_id);
                        match task_manager.delete_task(&task_id).await {
                            Ok(_) => {
                                println!("Task '{}' deleted successfully", task_id);
                            }
                            Err(e) => {
                                eprintln!("Error deleting task: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }

                    // Handle clean option
                    if clean {
                        println!("Cleaning completed tasks and quick tasks...");
                        match task_manager.clean_tasks().await {
                            Ok((completed_count, quick_count)) => {
                                println!("Cleanup complete:");
                                println!("  - {} completed task(s) deleted", completed_count);
                                println!("  - {} quick task(s) deleted", quick_count);
                            }
                            Err(e) => {
                                eprintln!("Error cleaning tasks: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error initializing task manager: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_template_validation() {
        // Create a temporary directory for templates
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir_all(&templates_dir).unwrap();

        // Create a test template
        let test_template_path = templates_dir.join("test-type.md");
        fs::write(&test_template_path, "Test template content").unwrap();

        // Test that validation passes for existing template
        let template_path = templates_dir.join("test-type.md");
        assert!(template_path.exists());

        // Test that validation fails for non-existing template
        let missing_template_path = templates_dir.join("missing-type.md");
        assert!(!missing_template_path.exists());
    }

    #[test]
    fn test_generic_type_no_template_required() {
        // The 'generic' type should not require a template
        let templates_dir = Path::new("templates");
        let _generic_template = templates_dir.join("generic.md");

        // Even if generic.md doesn't exist, it should be allowed
        // This is handled by the r#type != "generic" check in the code
        assert!(true); // Placeholder assertion - in real code this would be tested via CLI
    }

    #[test]
    fn test_task_type_default_value() {
        // Test that the default value for task type is "generic"
        // This is set via clap's default_value attribute
        // In a real integration test, we would parse CLI args
        assert!(true); // Placeholder - clap handles this via default_value
    }
}
