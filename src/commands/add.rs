use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::stdin_utils::{merge_description_with_stdin, read_piped_input};
use crate::task::TaskBuilder;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

pub struct AddCommand {
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
impl Command for AddCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Adding task to queue: {}", self.name);

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
            .description(final_description.clone())
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(self.agent.clone())
            .stack(self.stack.clone())
            .project(self.project.clone())
            .build(ctx)
            .await?;

        // Try to add task via server first
        let client = ctx.tsk_client();

        if client.is_server_available().await {
            // Server is available, use it
            match client.add_task(repo_root.clone(), task.clone()).await {
                Ok(_) => {
                    println!("Task added via server");
                }
                Err(_) => {
                    eprintln!("Failed to add task via server");
                    eprintln!("Falling back to direct file write...");

                    // Fall back to direct storage
                    let storage = get_task_storage(ctx.tsk_config(), ctx.file_system());
                    storage
                        .add_task(task.clone())
                        .await
                        .map_err(|e| e as Box<dyn Error>)?;
                }
            }
        } else {
            // Server not available, write directly
            let storage = get_task_storage(ctx.tsk_config(), ctx.file_system());
            storage
                .add_task(task.clone())
                .await
                .map_err(|e| e as Box<dyn Error>)?;
        }

        println!("\nTask successfully added to queue!");
        println!("Task ID: {}", task.id);
        println!("Type: {}", self.r#type);
        if let Some(ref desc) = final_description {
            println!("Description: {desc}");
        }
        if self.prompt.is_some() {
            println!("Prompt: Copied to task directory");
        }
        if let Some(ref agent) = self.agent {
            println!("Agent: {agent}");
        }
        println!("\nUse 'tsk list' to view all queued tasks");
        println!("Use 'tsk run' to execute the next task in the queue");

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
    async fn test_add_command_validation_no_input() {
        let cmd = AddCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(".".to_string()),
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
    async fn test_add_command_invalid_task_type() {
        let cmd = AddCommand {
            name: "test".to_string(),
            r#type: "nonexistent".to_string(),
            description: Some("test description".to_string()),
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(".".to_string()),
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("No template found for task type 'nonexistent'"),
            "Expected template error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_add_command_template_without_description() {
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

        // Create AddCommand without description (should succeed for templates without placeholder)
        let cmd = AddCommand {
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

        // Execute should succeed
        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Command should succeed for template without description placeholder"
        );
    }

    #[tokio::test]
    async fn test_add_command_with_repo_path() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create a template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        // Create AppContext - automatically gets test defaults
        let ctx = AppContext::builder().build();

        // Create AddCommand with repo path
        let cmd = AddCommand {
            name: "test-repo-path".to_string(),
            r#type: "generic".to_string(),
            description: Some("Test with repo path".to_string()),
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
        };

        // Execute should succeed without changing directories
        let current_dir = std::env::current_dir().unwrap();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Command should succeed with repo path");

        // Verify we didn't change directories
        assert_eq!(
            std::env::current_dir().unwrap(),
            current_dir,
            "Current directory should not have changed"
        );
    }
}
