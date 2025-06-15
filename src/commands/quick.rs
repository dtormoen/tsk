use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use crate::terminal::{restore_terminal_title, set_terminal_title};
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct QuickCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
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
            .instructions(self.instructions.clone())
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .build(ctx)
            .await?;

        if let Some(ref agent) = self.agent {
            println!("Agent: {}", agent);
        }
        println!("Timeout: {} minutes", self.timeout);

        // Update terminal title for the task
        set_terminal_title(&format!("TSK: {}", self.name));

        // Execute the task
        let task_manager = TaskManager::new(ctx)?;
        let result = task_manager
            .execute_queued_task(&task)
            .await
            .map_err(|e| e.message);

        // Restore terminal title
        restore_terminal_title();

        result?;

        Ok(())
    }
}
