use super::Command;
use crate::context::AppContext;
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

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

        // Create task using TaskBuilder
        let task = TaskBuilder::new()
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(self.description.clone())
            .instructions(self.instructions.clone())
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .quick(true) // This is a quick task
            .build(ctx)
            .await?;

        if let Some(ref agent) = self.agent {
            println!("Agent: {}", agent);
        }
        println!("Timeout: {} minutes", self.timeout);

        // Execute the task
        let task_manager = TaskManager::new(ctx)?;
        task_manager
            .execute_queued_task(&task)
            .await
            .map_err(|e| e.message)?;

        Ok(())
    }
}
