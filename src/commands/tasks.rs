use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct TasksCommand {
    pub delete: Option<String>,
    pub clean: bool,
}

#[async_trait]
impl Command for TasksCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Ensure at least one option is provided
        if self.delete.is_none() && !self.clean {
            return Err("Please specify either --delete <TASK_ID> or --clean".into());
        }

        let task_manager = TaskManager::with_storage(ctx)?;

        // Handle delete option
        if let Some(ref task_id) = self.delete {
            println!("Deleting task: {}", task_id);
            task_manager.delete_task(task_id).await?;
            println!("Task '{}' deleted successfully", task_id);
        }

        // Handle clean option
        if self.clean {
            println!("Cleaning completed tasks and quick tasks...");
            let (completed_count, quick_count) = task_manager.clean_tasks().await?;
            println!("Cleanup complete:");
            println!("  - {} completed task(s) deleted", completed_count);
            println!("  - {} quick task(s) deleted", quick_count);
        }

        Ok(())
    }
}
