use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::server::TskServer;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;

pub struct RunCommand {
    pub server: bool,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.server {
            // Run in server mode
            println!("Starting TSK server...");
            let server = TskServer::new(Arc::new(ctx.clone()));

            // Setup signal handlers for graceful shutdown
            let shutdown_signal = Arc::new(tokio::sync::Notify::new());
            let shutdown_signal_clone = shutdown_signal.clone();

            tokio::spawn(async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen for Ctrl+C");
                println!("\nReceived shutdown signal...");
                shutdown_signal_clone.notify_one();
            });

            // Run server until shutdown
            tokio::select! {
                result = server.run() => {
                    match result {
                        Ok(_) => {},
                        Err(e) => {
                            let error_msg = e.to_string();
                            eprintln!("Server error: {}", error_msg);
                            return Err(Box::new(std::io::Error::other(error_msg)));
                        }
                    }
                }
                _ = shutdown_signal.notified() => {
                    server.shutdown().await;
                }
            }

            println!("Server stopped");
            return Ok(());
        }

        // Run in client mode (execute current tasks and exit)
        let _repo_root = find_repository_root(Path::new("."))?;
        let storage = get_task_storage(ctx.xdg_directories(), ctx.file_system());
        let tasks = storage
            .list_tasks()
            .await
            .map_err(|e| e as Box<dyn Error>)?;

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
        let total_tasks = queued_tasks.len();
        let mut succeeded = 0;
        let mut failed = 0;

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
                    succeeded += 1;
                }
                Err(e) => {
                    eprintln!("Task failed: {}", e.message);
                    failed += 1;
                }
            }
        }

        println!("\n{}", "=".repeat(60));
        println!("All tasks processed!");
        println!("Use 'tsk list' to see the final status of all tasks");

        // Send summary notification
        ctx.notification_client()
            .notify_all_tasks_complete(total_tasks, succeeded, failed);

        Ok(())
    }
}
