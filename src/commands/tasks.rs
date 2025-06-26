use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct TasksCommand {
    pub delete: Option<String>,
    pub clean: bool,
    pub retry: Option<String>,
    pub edit: bool,
}

#[async_trait]
impl Command for TasksCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Ensure at least one option is provided
        if self.delete.is_none() && !self.clean && self.retry.is_none() {
            return Err(
                "Please specify either --delete <TASK_ID>, --clean, or --retry <TASK_ID>".into(),
            );
        }

        let task_manager = TaskManager::with_storage(ctx)?;

        // Handle delete option
        if let Some(ref task_id) = self.delete {
            println!("Deleting task: {task_id}");
            task_manager.delete_task(task_id).await?;
            println!("Task '{task_id}' deleted successfully");
        }

        // Handle clean option
        if self.clean {
            println!("Cleaning completed tasks...");
            let completed_count = task_manager.clean_tasks().await?;
            println!("Cleanup complete: {completed_count} completed task(s) deleted");
        }

        // Handle retry option
        if let Some(ref task_id) = self.retry {
            println!("Retrying task: {task_id}");
            let new_task_id = task_manager.retry_task(task_id, self.edit, ctx).await?;
            println!("Task retried successfully. New task ID: {new_task_id}");
        }

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
    async fn test_tasks_command_validation_no_options() {
        let cmd = TasksCommand {
            delete: None,
            clean: false,
            retry: None,
            edit: false,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Please specify either --delete"));
    }
}
