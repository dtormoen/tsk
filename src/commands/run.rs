use super::Command;
use crate::context::AppContext;
use crate::server::TskServer;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

pub struct RunCommand {
    pub server: bool,
    pub workers: u32,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.server {
            // Run in server mode
            println!("Starting TSK server with {} worker(s)...", self.workers);
            let server = TskServer::with_workers(Arc::new(ctx.clone()), self.workers);

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
                            eprintln!("Server error: {error_msg}");
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
        let storage = get_task_storage(ctx.tsk_config(), ctx.file_system());
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

        println!(
            "Found {} queued task(s) to run with {} worker(s)",
            queued_tasks.len(),
            self.workers
        );

        let task_manager = TaskManager::new(ctx)?;
        let total_tasks = queued_tasks.len();
        let succeeded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        if self.workers == 1 {
            // Sequential execution for single worker
            for task in queued_tasks {
                println!("\n{}", "=".repeat(60));
                println!("Running task: {} ({})", task.name, task.id);
                println!("Type: {}", task.task_type);
                println!("{}", "=".repeat(60));

                // Update terminal title for current task
                ctx.terminal_operations()
                    .set_title(&format!("TSK: {}", task.name));

                // Execute the task with automatic status updates
                match task_manager.execute_queued_task(&task).await {
                    Ok(_result) => {
                        println!("\nTask completed successfully");
                        succeeded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("Task failed: {}", e.message);
                        failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        } else {
            // Parallel execution for multiple workers
            use tokio::sync::Semaphore;
            use tokio::task::JoinSet;

            let semaphore = Arc::new(Semaphore::new(self.workers as usize));
            let mut active_tasks = JoinSet::new();

            for task in queued_tasks {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let task_manager = TaskManager::new(ctx)?;
                let succeeded = succeeded.clone();
                let failed = failed.clone();

                active_tasks.spawn(async move {
                    println!("\n{}", "=".repeat(60));
                    println!("Starting task: {} ({})", task.name, task.id);
                    println!("Type: {}", task.task_type);
                    println!("{}", "=".repeat(60));

                    // Execute the task with automatic status updates
                    match task_manager.execute_queued_task(&task).await {
                        Ok(_result) => {
                            println!("\nTask completed successfully: {}", task.name);
                            succeeded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(e) => {
                            eprintln!("\nTask failed: {} - {}", task.name, e.message);
                            failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    drop(permit);
                });
            }

            // Wait for all tasks to complete
            while active_tasks.join_next().await.is_some() {}
        }

        // Restore terminal title after all tasks complete
        ctx.terminal_operations().restore_title();

        println!("\n{}", "=".repeat(60));
        println!("All tasks processed!");
        println!("Use 'tsk list' to see the final status of all tasks");

        // Send summary notification
        let final_succeeded = succeeded.load(std::sync::atomic::Ordering::Relaxed);
        let final_failed = failed.load(std::sync::atomic::Ordering::Relaxed);
        ctx.notification_client().notify_all_tasks_complete(
            total_tasks,
            final_succeeded,
            final_failed,
        );

        Ok(())
    }
}
