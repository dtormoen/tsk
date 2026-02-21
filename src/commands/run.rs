use super::Command;
use crate::context::AppContext;
use crate::context::docker_client::DefaultDockerClient;
use crate::docker::DockerManager;
use crate::repo_utils::find_repository_root;
use crate::stdin_utils::{merge_description_with_stdin, read_piped_input};
use crate::task::TaskBuilder;
use crate::task_runner::TaskRunner;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct RunCommand {
    pub name: Option<String>,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
    pub no_network_isolation: bool,
    pub dind: bool,
    /// Optional Docker client override for dependency injection (used in tests)
    pub docker_client_override: Option<Arc<dyn crate::context::docker_client::DockerClient>>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Resolve name: use provided name or default to task type
        let name = self.name.clone().unwrap_or_else(|| self.r#type.clone());

        println!("Running task: {}", name);
        println!("Type: {}", self.r#type);

        // Parse comma-separated agents or use default
        let agents: Vec<String> = match &self.agent {
            Some(agent_str) => agent_str.split(',').map(|s| s.trim().to_string()).collect(),
            None => vec![crate::agent::AgentProvider::default_agent().to_string()],
        };

        // Validate all agents before creating any tasks
        for agent in &agents {
            if !crate::agent::AgentProvider::is_valid_agent(agent) {
                let available_agents = crate::agent::AgentProvider::list_agents().join(", ");
                return Err(format!(
                    "Unknown agent '{}'. Available agents: {}",
                    agent, available_agents
                )
                .into());
            }
        }

        // Run command only supports single agent (no multi-agent execution)
        if agents.len() > 1 {
            return Err(
                "Run command only supports a single agent. Use 'tsk add' for multi-agent tasks."
                    .into(),
            );
        }

        // Read from stdin if data is piped
        let piped_input = read_piped_input()?;

        // Merge piped input with CLI description (piped input takes precedence)
        let final_description = merge_description_with_stdin(self.description.clone(), piped_input);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create task using TaskBuilder
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(name.clone())
            .task_type(self.r#type.clone())
            .description(final_description)
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(Some(agents[0].clone()))
            .stack(self.stack.clone())
            .project(self.project.clone())
            .network_isolation(!self.no_network_isolation)
            .dind(if self.dind { Some(true) } else { None })
            .build(ctx)
            .await?;

        println!("Agent: {}", agents[0]);
        println!("Task ID: {}", task.id);

        // Update terminal title for the task
        ctx.terminal_operations()
            .set_title(&format!("TSK: {}", name));

        // Execute the task
        let docker_client: Arc<dyn crate::context::docker_client::DockerClient> =
            match &self.docker_client_override {
                Some(client) => Arc::clone(client),
                None => Arc::new(
                    DefaultDockerClient::new(&ctx.tsk_config().docker.container_engine)
                        .map_err(|e| -> Box<dyn Error> { e.into() })?,
                ),
            };
        let docker_manager = DockerManager::new(ctx, docker_client);
        let task_runner = TaskRunner::new(ctx, docker_manager);
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
            name: Some("test".to_string()),
            r#type: "generic".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            no_network_isolation: false,
            dind: false,
            docker_client_override: mock_docker_client(),
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg
                .contains("Either description or prompt file must be provided, or use edit mode"),
            "Expected validation error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_run_command_template_without_description() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = RunCommand {
            name: Some("test-ack".to_string()),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            no_network_isolation: false,
            dind: false,
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
            name: Some("test-multi".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: Some("codex,claude".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            no_network_isolation: false,
            dind: false,
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
            name: None,
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            no_network_isolation: false,
            dind: false,
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
            name: Some("test-single".to_string()),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: Some("codex".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            no_network_isolation: false,
            dind: false,
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
