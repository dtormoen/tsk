use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct CleanCommand;

#[async_trait]
impl Command for CleanCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Cleaning completed tasks...");
        let task_manager = TaskManager::new(ctx)?;
        let completed_count = task_manager.clean_tasks().await?;
        println!("Cleanup complete: {completed_count} completed task(s) deleted");
        Ok(())
    }
}
