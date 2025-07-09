use clap::{Args, Parser, Subcommand};

mod agent;
mod assets;
mod commands;
mod context;
mod docker;
mod git;
mod git_sync;
mod notifications;
mod repo_utils;
mod server;
mod storage;
mod task;
mod task_manager;
mod task_runner;
mod task_storage;
mod utils;

#[cfg(test)]
mod test_utils;

use commands::{
    AddCommand, CleanCommand, Command, DebugCommand, DeleteCommand, ListCommand, QuickCommand,
    RetryCommand, RunCommand,
    docker::DockerBuildCommand,
    proxy::ProxyStopCommand,
    server::{ServerRunCommand, ServerStopCommand},
    template::TemplateListCommand,
};
use context::AppContext;

#[derive(Parser)]
#[command(name = "tsk")]
#[command(author, version, about = "TSK - Task delegation to AI agents", long_about = None)]
#[command(help_template = r#"{name} {version}
{author-with-newline}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}
"#)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[command(about = "Task operations and configuration management", long_about = None)]
enum Commands {
    // Task Commands (implicit noun)
    /// Queue a task for later execution
    Add {
        /// Unique identifier for the task
        #[arg(short, long)]
        name: String,

        /// Task type (defaults to 'generic' if not specified)
        #[arg(short = 't', long, default_value = "generic")]
        r#type: String,

        /// Detailed description of what needs to be accomplished
        #[arg(short, long, conflicts_with = "prompt")]
        description: Option<String>,

        /// Path to prompt file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        prompt: Option<String>,

        /// Open the prompt file in $EDITOR after creation
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

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// List all queued tasks
    List,
    /// Run all queued tasks sequentially
    Run {
        /// Number of parallel workers for task execution
        #[arg(short, long, default_value = "1", value_parser = clap::value_parser!(u32).range(1..=32))]
        workers: u32,
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
        #[arg(short, long, conflicts_with = "prompt")]
        description: Option<String>,

        /// Path to prompt file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        prompt: Option<String>,

        /// Open the prompt file in $EDITOR after creation
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

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Launch a Docker container for interactive debugging
    Debug {
        /// Unique identifier for the debug session
        #[arg(short, long)]
        name: String,

        /// Specific agent to use (defaults to claude-code)
        #[arg(short, long)]
        agent: Option<String>,

        /// Technology stack for Docker image (e.g., rust, python, node)
        #[arg(long)]
        tech_stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete all completed tasks and all quick tasks
    Clean,
    /// Delete a specific task by ID
    Delete {
        /// Task ID to delete
        task_id: String,
    },
    /// Retry a task by creating a new task with the same instructions
    Retry {
        /// Task ID to retry
        task_id: String,
        /// Open the prompt file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,
    },
    // Configuration and Infrastructure Commands
    /// Server operations - manage the TSK server daemon
    Server(ServerArgs),
    /// Docker operations - build and manage TSK Docker images
    Docker(DockerArgs),
    /// Proxy operations - manage the TSK proxy container
    Proxy(ProxyArgs),
    /// Template operations - manage task templates
    Template(TemplateArgs),
}

#[derive(Args)]
#[command(about = "Manage the TSK server daemon")]
struct ServerArgs {
    #[command(subcommand)]
    command: ServerCommands,
}

#[derive(Subcommand)]
enum ServerCommands {
    /// Start the TSK server daemon
    Run {
        /// Number of parallel workers for task execution
        #[arg(short, long, default_value = "1", value_parser = clap::value_parser!(u32).range(1..=32))]
        workers: u32,
    },
    /// Stop the running TSK server
    Stop,
}

#[derive(Args)]
#[command(about = "Build and manage TSK Docker images")]
struct DockerArgs {
    #[command(subcommand)]
    command: DockerCommands,
}

#[derive(Subcommand)]
enum DockerCommands {
    /// Build TSK Docker images
    Build {
        /// Build without using Docker's cache
        #[arg(long)]
        no_cache: bool,

        /// Technology stack (e.g., rust, python, node)
        #[arg(long)]
        tech_stack: Option<String>,

        /// Agent (e.g., claude-code, aider)
        #[arg(long)]
        agent: Option<String>,

        /// Project name
        #[arg(long)]
        project: Option<String>,

        /// Print the resolved Dockerfile without building
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Args)]
#[command(about = "Manage the TSK proxy container")]
struct ProxyArgs {
    #[command(subcommand)]
    command: ProxyCommands,
}

#[derive(Subcommand)]
enum ProxyCommands {
    /// Stop the TSK proxy container
    Stop,
}

#[derive(Args)]
#[command(about = "Manage task templates")]
struct TemplateArgs {
    #[command(subcommand)]
    command: TemplateCommands,
}

#[derive(Subcommand)]
enum TemplateCommands {
    /// List available task templates and their sources
    List,
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
            prompt,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
            repo,
        } => Box::new(AddCommand {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
            repo,
        }),
        Commands::Quick {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
            repo,
        } => Box::new(QuickCommand {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            timeout,
            tech_stack,
            project,
            repo,
        }),
        Commands::Debug {
            name,
            agent,
            tech_stack,
            project,
            repo,
        } => Box::new(DebugCommand {
            name,
            agent,
            tech_stack,
            project,
            repo,
        }),
        Commands::List => Box::new(ListCommand),
        Commands::Run { workers } => Box::new(RunCommand {
            server: false,
            workers,
        }),
        Commands::Clean => Box::new(CleanCommand),
        Commands::Delete { task_id } => Box::new(DeleteCommand { task_id }),
        Commands::Retry { task_id, edit } => Box::new(RetryCommand { task_id, edit }),
        Commands::Server(server_args) => match server_args.command {
            ServerCommands::Run { workers } => Box::new(ServerRunCommand { workers }),
            ServerCommands::Stop => Box::new(ServerStopCommand),
        },
        Commands::Docker(docker_args) => match docker_args.command {
            DockerCommands::Build {
                no_cache,
                tech_stack,
                agent,
                project,
                dry_run,
            } => Box::new(DockerBuildCommand {
                no_cache,
                tech_stack,
                agent,
                project,
                dry_run,
            }),
        },
        Commands::Proxy(proxy_args) => match proxy_args.command {
            ProxyCommands::Stop => Box::new(ProxyStopCommand),
        },
        Commands::Template(template_args) => match template_args.command {
            TemplateCommands::List => Box::new(TemplateListCommand),
        },
    };

    if let Err(e) = command.execute(&app_context).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
