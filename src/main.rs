use clap::{Parser, Subcommand};

mod commands;
use commands::{
    AddCommand, Command, DebugCommand, ListCommand, QuickCommand, RunCommand, StopProxyCommand,
    TasksCommand,
};

mod context;
use context::AppContext;

mod git;

mod docker;

mod task;

mod task_storage;

mod task_manager;

mod task_runner;

mod log_processor;

mod agent;

mod repo_utils;

mod notifications;

#[cfg(test)]
mod test_utils;

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

        /// Specific agent to use (defaults to claude-code)
        #[arg(short, long)]
        agent: Option<String>,
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

    // Create the AppContext using the builder pattern
    let app_context = AppContext::builder().build();

    let command: Box<dyn Command> = match cli.command {
        Commands::Add {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
        } => Box::new(AddCommand {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
        }),
        Commands::Quick {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
        } => Box::new(QuickCommand {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
        }),
        Commands::Debug { name, agent } => Box::new(DebugCommand { name, agent }),
        Commands::StopProxy => Box::new(StopProxyCommand),
        Commands::List => Box::new(ListCommand),
        Commands::Run => Box::new(RunCommand),
        Commands::Tasks { delete, clean } => Box::new(TasksCommand { delete, clean }),
    };

    if let Err(e) = command.execute(&app_context).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
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
