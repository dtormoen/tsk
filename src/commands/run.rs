use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct RunCommand;

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let repo_root = find_repository_root(Path::new("."))?;
        let storage = get_task_storage(&repo_root, ctx.file_system());
        let tasks = storage.list_tasks().await?;

        let queued_tasks: Vec<Task> = tasks
            .into_iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .collect();

        if queued_tasks.is_empty() {
            println!("No queued tasks to run");
            return Ok(());
        }

        println!("Found {} queued task(s) to run", queued_tasks.len());

        let task_manager = TaskManager::with_storage(ctx)?;

        for task in queued_tasks {
            println!("\n{}", "=".repeat(60));
            println!("Running task: {} ({})", task.name, task.id);
            println!("Type: {}", task.task_type);
            if let Some(ref desc) = task.description {
                println!("Description: {}", desc);
            }
            println!("{}", "=".repeat(60));

            // Execute the task with automatic status updates
            match task_manager.execute_queued_task(&task).await {
                Ok(_result) => {
                    println!("\nTask completed successfully");
                }
                Err(e) => {
                    eprintln!("Task failed: {}", e.message);
                }
            }
        }

        println!("\n{}", "=".repeat(60));
        println!("All tasks processed!");
        println!("Use 'tsk list' to see the final status of all tasks");

        Ok(())
    }
}
