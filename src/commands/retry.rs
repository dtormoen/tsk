use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct RetryCommand {
    pub task_id: String,
    pub edit: bool,
}

#[async_trait]
impl Command for RetryCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Retrying task: {}", self.task_id);
        let task_manager = TaskManager::with_storage(ctx)?;
        let new_task_id = task_manager
            .retry_task(&self.task_id, self.edit, ctx)
            .await?;
        println!("Task retried successfully. New task ID: {new_task_id}");
        Ok(())
    }
}
