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

pub struct RunCommand {
    pub task_args: TaskArgs,
    /// Optional Docker client override for dependency injection (used in tests)
    pub docker_client_override: Option<Arc<dyn crate::context::docker_client::DockerClient>>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let args = &self.task_args;
        let name = args.resolved_name();

        let agents = args.parse_and_validate_agents()?;

        // Run command only supports single agent (no multi-agent execution)
        if agents.len() > 1 {
            return Err(
                "Run command only supports a single agent. Use 'tsk add' for multi-agent tasks."
                    .into(),
            );
        }

        let prompt = args.resolve_prompt()?;
        let repo_root = args.resolve_repo_root()?;

        let task = args
            .configure_builder(repo_root, name.clone(), Some(agents[0].clone()), prompt)
            .build(ctx)
            .await?;

        println!(
            "Running {} ({}, {}, {})",
            task.id, task.task_type, task.stack, task.agent
        );

        // Update terminal title for the task
        ctx.terminal_operations().set_title(&format!("TSK: {name}"));

        // Execute the task
        let docker_client: Arc<dyn crate::context::docker_client::DockerClient> =
            match &self.docker_client_override {
                Some(client) => Arc::clone(client),
                None => Arc::new(
                    DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                        .map_err(|e| -> Box<dyn Error> { e.into() })?,
                ),
            };
        let docker_manager = DockerManager::new(ctx, docker_client.clone(), None);
        let task_runner = TaskRunner::new(ctx, docker_manager, None);

        // Set up signal handler for Ctrl+C cancellation
        let storage = ctx.task_storage();
        let task_id = task.id.clone();
        let container_name = format!("tsk-{}", task.id);
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
    use crate::test_utils::NoOpDockerClient;

    fn mock_docker_client() -> Option<Arc<dyn crate::context::docker_client::DockerClient>> {
        Some(Arc::new(NoOpDockerClient))
    }

    fn create_test_context() -> AppContext {
        AppContext::builder().build()
    }

    #[tokio::test]
    async fn test_run_command_validation_no_input() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let cmd = RunCommand {
            task_args: TaskArgs {
                name: Some("test".to_string()),
                r#type: "generic".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: mock_docker_client(),
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Either prompt or prompt file must be provided, or use edit mode"),
            "Expected validation error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_run_command_template_without_prompt() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = RunCommand {
            task_args: TaskArgs {
                name: Some("test-ack".to_string()),
                r#type: "ack".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: mock_docker_client(),
        };

        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Should succeed for template without placeholder: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_run_command_rejects_multiple_agents() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let cmd = RunCommand {
            task_args: TaskArgs {
                name: Some("test-multi".to_string()),
                r#type: "generic".to_string(),
                description: Some("Test description".to_string()),
                agent: Some("codex,claude".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: mock_docker_client(),
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;

        assert!(result.is_err(), "Should reject multiple agents");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Run command only supports a single agent"),
            "Error message should explain limitation: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_run_command_name_defaults_to_type() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = RunCommand {
            task_args: TaskArgs {
                r#type: "ack".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: mock_docker_client(),
        };

        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Should succeed with name defaulting to type: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_run_command_with_single_agent() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        test_repo
            .create_file(".tsk/templates/ack.md", "Say ack and exit.")
            .unwrap();

        let cmd = RunCommand {
            task_args: TaskArgs {
                name: Some("test-single".to_string()),
                r#type: "ack".to_string(),
                agent: Some("codex".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: mock_docker_client(),
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;

        assert!(
            result.is_ok(),
            "Should succeed with single agent: {:?}",
            result.err()
        );
    }
}
