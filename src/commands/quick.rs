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
    pub instructions: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
    pub tech_stack: Option<String>,
    pub project: Option<String>,
}

#[async_trait]
impl Command for QuickCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Executing quick task: {}", self.name);
        println!("Type: {}", self.r#type);

        // Find repository root
        let repo_root = find_repository_root(Path::new("."))?;

        // Create task using TaskBuilder
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(self.description.clone())
            .instructions_file(self.instructions.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .tech_stack(self.tech_stack.clone())
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
    use crate::test_utils::{NoOpDockerClient, NoOpTskClient};
    use std::sync::Arc;

    fn create_test_context() -> AppContext {
        AppContext::builder()
            .with_docker_client(Arc::new(NoOpDockerClient))
            .with_tsk_client(Arc::new(NoOpTskClient))
            .build()
    }

    #[tokio::test]
    async fn test_quick_command_validation_no_input() {
        let cmd = QuickCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            instructions: None,
            edit: false,
            agent: None,
            timeout: 30,
            tech_stack: None,
            project: None,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(
            "Either description or instructions file must be provided, or use edit mode"
        ));
    }
}
