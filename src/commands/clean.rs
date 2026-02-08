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
        let result = task_manager.clean_tasks().await?;
        println!(
            "Cleanup complete: {} completed task(s) deleted",
            result.deleted
        );
        if result.skipped > 0 {
            println!(
                "Skipped {} task(s) with pending child tasks",
                result.skipped
            );
        }
        Ok(())
    }
}
