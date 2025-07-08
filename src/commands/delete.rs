use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct DeleteCommand {
    pub task_id: String,
}

#[async_trait]
impl Command for DeleteCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Deleting task: {}", self.task_id);
        let task_manager = TaskManager::with_storage(ctx)?;
        task_manager.delete_task(&self.task_id).await?;
        println!("Task '{}' deleted successfully", self.task_id);
        Ok(())
    }
}
