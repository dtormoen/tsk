use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

pub struct QuickCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
}

#[async_trait]
impl Command for QuickCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Executing quick task: {}", self.name);
        println!("Type: {}", self.r#type);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create task using TaskBuilder
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(self.description.clone())
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .stack(self.stack.clone())
            .project(self.project.clone())
            .build(ctx)
            .await?;

        if let Some(ref agent) = self.agent {
            println!("Agent: {agent}");
        }
        println!("Timeout: {} minutes", self.timeout);

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
    async fn test_quick_command_validation_no_input() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let cmd = QuickCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            timeout: 30,
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
    async fn test_quick_command_template_without_description() {
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

        // Create QuickCommand without description (should succeed for templates without placeholder)
        let cmd = QuickCommand {
            name: "test-ack".to_string(),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            timeout: 30,
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
}
