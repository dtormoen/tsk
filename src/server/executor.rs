use crate::context::AppContext;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_storage::TaskStorage;
use crate::terminal::{restore_terminal_title, set_terminal_title};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

/// Task executor that runs tasks sequentially
pub struct TaskExecutor {
    app_context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    running: Arc<Mutex<bool>>,
}

impl TaskExecutor {
    /// Create a new task executor
    pub fn new(app_context: Arc<AppContext>, storage: Arc<Mutex<Box<dyn TaskStorage>>>) -> Self {
        Self {
            app_context,
            storage,
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Start executing tasks from the queue
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut running = self.running.lock().await;
        if *running {
            return Err("Executor is already running".into());
        }
        *running = true;
        drop(running);

        println!("Task executor started");

        // Set initial idle title
        set_terminal_title("TSK Server Idle");

        loop {
            // Check if we should continue running
            if !*self.running.lock().await {
                println!("Task executor stopping");
                restore_terminal_title();
                break;
            }

            // Get the next queued task
            let storage = self.storage.lock().await;
            let tasks = storage.list_tasks().await?;
            drop(storage);

            let queued_task = tasks.into_iter().find(|t| t.status == TaskStatus::Queued);

            match queued_task {
                Some(task) => {
                    println!("Executing task: {} ({})", task.name, task.id);

                    // Update terminal title to show current task
                    set_terminal_title(&format!("TSK: {}", task.name));

                    // Update task status to running
                    let mut running_task = task.clone();
                    running_task.status = TaskStatus::Running;
                    running_task.started_at = Some(chrono::Utc::now());

                    let storage = self.storage.lock().await;
                    storage.update_task(running_task.clone()).await?;
                    drop(storage);

                    // Execute the task
                    let execution_result = self.execute_task(&running_task).await;

                    match execution_result {
                        Ok(_) => {
                            println!("Task completed successfully: {}", running_task.id);

                            // Update task status to complete
                            let mut completed_task = running_task.clone();
                            completed_task.status = TaskStatus::Complete;
                            completed_task.completed_at = Some(chrono::Utc::now());

                            let storage = self.storage.lock().await;
                            storage.update_task(completed_task).await?;
                        }
                        Err(e) => {
                            let error_message = e.to_string();
                            eprintln!("Task failed: {} - {}", running_task.id, error_message);

                            // Update task status to failed
                            let mut failed_task = running_task.clone();
                            failed_task.status = TaskStatus::Failed;
                            failed_task.completed_at = Some(chrono::Utc::now());
                            failed_task.error_message = Some(error_message);

                            let storage = self.storage.lock().await;
                            storage.update_task(failed_task).await?;
                        }
                    }

                    // Restore idle title after task completion
                    set_terminal_title("TSK Server Idle");
                }
                None => {
                    // No tasks to execute, wait a bit before checking again
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }

        Ok(())
    }

    /// Stop the executor
    pub async fn stop(&self) {
        *self.running.lock().await = false;
    }

    /// Execute a single task
    async fn execute_task(
        &self,
        task: &Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create a task manager with the current context
        let task_manager = TaskManager::with_storage(&self.app_context)?;

        // Execute the task
        let result = task_manager.execute_queued_task(task).await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(e.message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::tests::MockFileSystem;
    use crate::storage::XdgDirectories;
    use crate::task_storage::get_task_storage;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_executor_lifecycle() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().join("data"));
        std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path().join("runtime"));

        let xdg = Arc::new(XdgDirectories::new().unwrap());
        xdg.ensure_directories().unwrap();

        let fs = Arc::new(MockFileSystem::new());
        let storage = Arc::new(Mutex::new(get_task_storage(xdg.clone(), fs.clone())));

        let app_context = crate::context::AppContext::builder()
            .with_file_system(fs)
            .with_xdg_directories(xdg)
            .build();

        let executor = TaskExecutor::new(Arc::new(app_context), storage);

        // Test that executor can be started and stopped
        assert!(!*executor.running.lock().await);

        // Start executor in background
        let executor_clone = Arc::new(executor);
        let exec_handle = {
            let exec = executor_clone.clone();
            tokio::spawn(async move {
                let _ = exec.start().await;
            })
        };

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(*executor_clone.running.lock().await);

        // Stop executor
        executor_clone.stop().await;
        let _ = exec_handle.await;

        assert!(!*executor_clone.running.lock().await);
    }
}
