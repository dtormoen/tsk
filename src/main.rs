use clap::{Parser, Subcommand};

use tsk::commands::{
    AddCommand, Command, DebugCommand, DockerBuildCommand, ListCommand, QuickCommand, RunCommand,
    StopProxyCommand, StopServerCommand, TasksCommand, TemplatesCommand,
};
use tsk::context::AppContext;

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

        /// Technology stack for Docker image (e.g., rust, python, node)
        #[arg(long)]
        tech_stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,
    },
    /// List all queued tasks
    List,
    /// Run all queued tasks sequentially
    Run {
        /// Run in server mode (start daemon and keep running)
        #[arg(short, long)]
        server: bool,
    },
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

        /// Technology stack for Docker image (e.g., rust, python, node)
        #[arg(long)]
        tech_stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,
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
    /// Stop the TSK server
    StopServer,
    /// Manage tasks in the task list
    Tasks {
        /// Delete a specific task by ID
        #[arg(short, long, value_name = "TASK_ID", conflicts_with_all = ["clean", "retry"])]
        delete: Option<String>,

        /// Delete all completed tasks and all quick tasks
        #[arg(short, long, conflicts_with_all = ["delete", "retry"])]
        clean: bool,

        /// Retry a task by creating a new task with the same instructions
        #[arg(short, long, value_name = "TASK_ID", conflicts_with_all = ["delete", "clean"])]
        retry: Option<String>,

        /// Open the instructions file in $EDITOR after creation (only with --retry)
        #[arg(short, long, requires = "retry")]
        edit: bool,
    },
    /// Build TSK Docker images
    DockerBuild {
        /// Build without using Docker's cache
        #[arg(long)]
        no_cache: bool,

        /// Technology stack (e.g., rust, python, node)
        #[arg(long)]
        tech_stack: Option<String>,

        /// Agent (e.g., claude, aider)
        #[arg(long)]
        agent: Option<String>,

        /// Project name
        #[arg(long)]
        project: Option<String>,

        /// Build legacy tsk/base and tsk/proxy images
        #[arg(long)]
        legacy: bool,
    },
    /// List available task templates and their sources
    Templates,
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
            tech_stack,
            project,
        } => Box::new(AddCommand {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
        }),
        Commands::Quick {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
        } => Box::new(QuickCommand {
            name,
            r#type,
            description,
            instructions,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
        }),
        Commands::Debug { name, agent } => Box::new(DebugCommand { name, agent }),
        Commands::StopProxy => Box::new(StopProxyCommand),
        Commands::StopServer => Box::new(StopServerCommand),
        Commands::List => Box::new(ListCommand),
        Commands::Run { server } => Box::new(RunCommand { server }),
        Commands::Tasks {
            delete,
            clean,
            retry,
            edit,
        } => Box::new(TasksCommand {
            delete,
            clean,
            retry,
            edit,
        }),
        Commands::DockerBuild {
            no_cache,
            tech_stack,
            agent,
            project,
            legacy,
        } => Box::new(DockerBuildCommand {
            no_cache,
            tech_stack,
            agent,
            project,
            legacy,
        }),
        Commands::Templates => Box::new(TemplatesCommand),
    };

    if let Err(e) = command.execute(&app_context).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
