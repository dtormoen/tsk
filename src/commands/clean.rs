use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct CleanCommand;

#[async_trait]
impl Command for CleanCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let task_manager = TaskManager::new(ctx)?;
        let result = task_manager.clean_tasks(false, None).await?;
        println!("Cleaned {} task(s)", result.deleted);
        if result.skipped > 0 {
            println!("Skipped {} with pending children", result.skipped);
        }
        Ok(())
    }
}
