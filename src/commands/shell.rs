use super::Command;
use super::task_args::TaskArgs;
use crate::context::AppContext;
use crate::context::docker_client::DefaultDockerClient;
use crate::docker::DockerManager;
use crate::task_runner::TaskRunner;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};

/// Command for starting interactive shell sessions.
///
/// Creates a sandbox container with an agent for interactive use.
/// Allows developers to explore and test in isolated Docker containers.
pub struct ShellCommand {
    pub task_args: TaskArgs,
}

#[async_trait]
impl Command for ShellCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let args = &self.task_args;
        let name = args.resolved_name();

        let prompt = args.resolve_prompt()?;
        let repo_root = args.resolve_repo_root()?;

        let task = args
            .configure_builder(repo_root, name.clone(), args.agent.clone(), prompt)
            .with_interactive(true) // Shell sessions are always interactive
            .build(ctx)
            .await?;

        println!("Shell {} ({}, {})", task.id, task.stack, task.agent);

        // Update terminal title for the shell session
        ctx.terminal_operations()
            .set_title(&format!("TSK Shell: {name}"));

        // Execute the task
        let docker_client: Arc<dyn crate::context::docker_client::DockerClient> = Arc::new(
            DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                .map_err(|e| -> Box<dyn Error> { e.into() })?,
        );
        let docker_manager = DockerManager::new(ctx, docker_client.clone(), None);
        let task_runner = TaskRunner::new(ctx, docker_manager, None);

        // Set up signal handler for cancellation.
        // During the active interactive session, Ctrl+C is forwarded to the container
        // via raw mode stdin, so this handler primarily catches SIGTERM signals
        // and Ctrl+C during setup/teardown windows.
        let storage = ctx.task_storage();
        let task_id = task.id.clone();
        let container_name = format!("tsk-interactive-{}", task.id);
        let cancel_client = docker_client.clone();
        let cancel_storage = storage.clone();

        tokio::spawn(async move {
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
            let _ = cancel_storage.mark_cancelled(&task_id).await;
            let _ = cancel_client.kill_container(&container_name).await;
        });

        let result = task_runner
            .store_and_run(&task)
            .await
            .map_err(|e| e.message);

        // Restore terminal title
        ctx.terminal_operations().restore_title();

        result?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_command_structure() {
        let cmd = ShellCommand {
            task_args: TaskArgs {
                name: Some("test-shell".to_string()),
                r#type: "shell".to_string(),
                description: Some("Test description".to_string()),
                agent: Some("claude".to_string()),
                ..Default::default()
            },
        };

        let args = &cmd.task_args;
        assert_eq!(args.resolved_name(), "test-shell");
        assert_eq!(args.r#type, "shell");
        assert_eq!(args.description, Some("Test description".to_string()));
        assert_eq!(args.prompt, None);
        assert!(!args.edit);
        assert_eq!(args.agent, Some("claude".to_string()));
        assert_eq!(args.stack, None);
        assert_eq!(args.project, None);
        assert_eq!(args.repo, None);
    }
}
