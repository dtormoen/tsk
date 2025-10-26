use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::stdin_utils::{merge_description_with_stdin, read_piped_input};
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

pub struct RunCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Running task: {}", self.name);
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
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(final_description)
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(Some(agents[0].clone()))
            .stack(self.stack.clone())
            .project(self.project.clone())
            .build(ctx)
            .await?;

        println!("Agent: {}", agents[0]);

        // Update terminal title for the task
        ctx.terminal_operations()
            .set_title(&format!("TSK: {}", self.name));

        // Execute the task
        let task_manager = TaskManager::new(ctx)?;
        let result = task_manager
            .execute_queued_task(&task)
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

    fn create_test_context() -> AppContext {
        // Automatically gets test defaults: NoOpDockerClient, NoOpTskClient, etc.
        AppContext::builder().build()
    }

    #[tokio::test]
    async fn test_run_command_validation_no_input() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let cmd = RunCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
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

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create template file without {{DESCRIPTION}} placeholder
        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        // Create AppContext - automatically gets test defaults
        let ctx = AppContext::builder().build();

        // Create RunCommand without description (should succeed for templates without placeholder)
        let cmd = RunCommand {
            name: "test-ack".to_string(),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
        };

        // Execute should succeed for templates without {{DESCRIPTION}} placeholder
        // The NoOpDockerClient simulates successful execution
        let result = cmd.execute(&ctx).await;

        // The test verifies that templates without {{DESCRIPTION}} placeholder
        // don't require a description to be provided
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
            name: "test-multi".to_string(),
            r#type: "generic".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: Some("codex,claude".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
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
    async fn test_run_command_with_single_agent() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        test_repo
            .create_file(".tsk/templates/ack.md", "Say ack and exit.")
            .unwrap();

        let cmd = RunCommand {
            name: "test-single".to_string(),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: Some("codex".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
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
