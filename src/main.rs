#![deny(clippy::disallowed_methods)]
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
mod repository;
mod server;
mod stdin_utils;
mod storage;
mod task;
mod task_builder;
mod task_manager;
mod task_runner;
mod task_storage;
mod utils;

#[cfg(test)]
mod test_utils;

use commands::{
    AddCommand, CleanCommand, Command, DeleteCommand, ListCommand, RetryCommand, RunCommand,
    ShellCommand,
    docker::DockerBuildCommand,
    proxy::ProxyStopCommand,
    server::{ServerStartCommand, ServerStopCommand},
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
    /// Immediately run a task in a sandbox container synchronously
    Run {
        /// Unique identifier for the task
        #[arg(short, long)]
        name: String,

        /// Task type (defaults to 'generic' if not specified)
        #[arg(short = 't', long, default_value = "generic")]
        r#type: String,

        /// Detailed description of what needs to be accomplished (can also be piped via stdin)
        #[arg(short, long, conflicts_with = "prompt")]
        description: Option<String>,

        /// Path to prompt file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        prompt: Option<String>,

        /// Open the prompt file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,

        /// Specific agent to use (claude, codex)
        #[arg(short, long)]
        agent: Option<String>,

        /// Stack for Docker image (e.g., rust, python, node)
        #[arg(long)]
        stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Launch a sandbox container with an agent for interactive use
    Shell {
        /// Unique identifier for the shell session
        #[arg(short, long, default_value = "shell")]
        name: String,

        /// Task type (defaults to 'shell' if not specified)
        #[arg(short = 't', long, default_value = "shell")]
        r#type: String,

        /// Detailed description of what needs to be accomplished (can also be piped via stdin)
        #[arg(short, long, conflicts_with = "prompt")]
        description: Option<String>,

        /// Path to prompt file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        prompt: Option<String>,

        /// Open the prompt file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,

        /// Specific agent to use (defaults to claude)
        #[arg(short, long)]
        agent: Option<String>,

        /// Stack for Docker image (e.g., rust, python, node)
        #[arg(long, alias = "tech-stack")]
        stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Queue a task for later execution by the TSK server
    Add {
        /// Unique identifier for the task
        #[arg(short, long)]
        name: String,

        /// Task type (defaults to 'generic' if not specified)
        #[arg(short = 't', long, default_value = "generic")]
        r#type: String,

        /// Detailed description of what needs to be accomplished (can also be piped via stdin)
        #[arg(short, long, conflicts_with = "prompt")]
        description: Option<String>,

        /// Path to prompt file to pass to the agent
        #[arg(short, long, conflicts_with = "description")]
        prompt: Option<String>,

        /// Open the prompt file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,

        /// Specific agent to use (claude, codex)
        #[arg(short, long)]
        agent: Option<String>,

        /// Stack for Docker image (e.g., rust, python, node)
        #[arg(long)]
        stack: Option<String>,

        /// Project name for Docker image
        #[arg(long)]
        project: Option<String>,

        /// Path to the git repository (defaults to current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Start or stop the TSK server daemon that runs queued tasks in containers
    Server(ServerArgs),
    /// List all queued tasks
    List,
    /// Delete all completed tasks
    Clean,
    /// Delete one or more tasks by ID
    Delete {
        /// Task IDs to delete
        task_ids: Vec<String>,
    },
    /// Retry one or more tasks by creating new tasks with the same instructions
    Retry {
        /// Task IDs to retry
        task_ids: Vec<String>,
        /// Open the prompt file in $EDITOR after creation
        #[arg(short, long)]
        edit: bool,
    },
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
    Start {
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

        /// Stack (e.g., rust, python, node)
        #[arg(long)]
        stack: Option<String>,

        /// Agent (e.g., claude, codex)
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
            stack,
            project,
            repo,
        } => Box::new(AddCommand {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            stack,
            project,
            repo,
        }),
        Commands::Run {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            stack,
            project,
            repo,
        } => Box::new(RunCommand {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            stack,
            project,
            repo,
        }),
        Commands::Shell {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            stack,
            project,
            repo,
        } => Box::new(ShellCommand {
            name,
            r#type,
            description,
            prompt,
            edit,
            agent,
            stack,
            project,
            repo,
        }),
        Commands::List => Box::new(ListCommand),
        Commands::Clean => Box::new(CleanCommand),
        Commands::Delete { task_ids } => Box::new(DeleteCommand { task_ids }),
        Commands::Retry { task_ids, edit } => Box::new(RetryCommand { task_ids, edit }),
        Commands::Server(server_args) => match server_args.command {
            ServerCommands::Start { workers } => Box::new(ServerStartCommand { workers }),
            ServerCommands::Stop => Box::new(ServerStopCommand),
        },
        Commands::Docker(docker_args) => match docker_args.command {
            DockerCommands::Build {
                no_cache,
                stack,
                agent,
                project,
                dry_run,
            } => Box::new(DockerBuildCommand {
                no_cache,
                stack,
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
