use crate::context::AppContext;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_storage::TaskStorage;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{Duration, sleep};

/// Task executor that runs tasks in parallel with configurable workers
pub struct TaskExecutor {
    context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    running: Arc<Mutex<bool>>,
    workers: u32,
}

impl TaskExecutor {
    /// Create a new task executor with default single worker
    pub fn new(context: Arc<AppContext>, storage: Arc<Mutex<Box<dyn TaskStorage>>>) -> Self {
        Self::with_workers(context, storage, 1)
    }

    /// Create a new task executor with specified number of workers
    pub fn with_workers(
        context: Arc<AppContext>,
        storage: Arc<Mutex<Box<dyn TaskStorage>>>,
        workers: u32,
    ) -> Self {
        Self {
            context,
            storage,
            running: Arc::new(Mutex::new(false)),
            workers,
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

        println!("Task executor started with {} worker(s)", self.workers);

        // Set initial idle title
        self.context
            .terminal_operations()
            .set_title(&format!("TSK Server Idle (0/{} workers)", self.workers));

        // Create a semaphore to limit concurrent tasks
        let semaphore = Arc::new(Semaphore::new(self.workers as usize));

        // Track active tasks
        let mut active_tasks = JoinSet::new();

        loop {
            // Check if we should continue running
            if !*self.running.lock().await {
                println!("Task executor stopping, waiting for active tasks to complete...");

                // Wait for all active tasks to complete
                while active_tasks.join_next().await.is_some() {}

                self.context.terminal_operations().restore_title();
                break;
            }

            // Clean up completed tasks
            while let Some(result) = active_tasks.try_join_next() {
                if let Err(e) = result {
                    eprintln!("Task join error: {e}");
                }
            }

            // Try to acquire a permit to run a new task
            let permit = semaphore.clone().try_acquire_owned();

            if permit.is_err() {
                // All workers are busy, wait a bit before checking again
                sleep(Duration::from_millis(100)).await;
                continue;
            }

            // We have a permit, look for a queued task
            let storage = self.storage.lock().await;
            let tasks = storage.list_tasks().await?;
            drop(storage);

            let queued_task = tasks.into_iter().find(|t| t.status == TaskStatus::Queued);

            match queued_task {
                Some(task) => {
                    println!("Starting task: {} ({})", task.name, task.id);

                    // Update task status to running
                    let mut running_task = task.clone();
                    running_task.status = TaskStatus::Running;
                    running_task.started_at = Some(chrono::Utc::now());

                    let storage = self.storage.lock().await;
                    storage
                        .update_task_status(
                            &running_task.id,
                            TaskStatus::Running,
                            Some(chrono::Utc::now()),
                            None,
                            None,
                        )
                        .await?;
                    drop(storage);

                    // Spawn task execution
                    let context = self.context.clone();
                    let storage = self.storage.clone();
                    let _permit = permit.unwrap(); // We checked is_err() above

                    active_tasks.spawn(async move {
                        // Execute the task
                        let execution_result =
                            Self::execute_single_task(&context, &running_task).await;

                        match execution_result {
                            Ok(_) => {
                                println!("Task completed successfully: {}", running_task.id);

                                // Update task status to complete
                                let storage = storage.lock().await;
                                let _ = storage
                                    .update_task_status(
                                        &running_task.id,
                                        TaskStatus::Complete,
                                        None,
                                        Some(chrono::Utc::now()),
                                        None,
                                    )
                                    .await;
                                drop(storage);
                            }
                            Err(e) => {
                                let error_message = e.to_string();
                                eprintln!("Task failed: {} - {}", running_task.id, error_message);

                                // Update task status to failed
                                let storage = storage.lock().await;
                                let _ = storage
                                    .update_task_status(
                                        &running_task.id,
                                        TaskStatus::Failed,
                                        None,
                                        Some(chrono::Utc::now()),
                                        Some(error_message),
                                    )
                                    .await;
                                drop(storage);
                            }
                        }

                        // Permit is automatically dropped here, releasing the semaphore
                    });
                }
                None => {
                    // No tasks to execute, release the permit and wait
                    drop(permit.unwrap());
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
    async fn execute_single_task(
        context: &AppContext,
        task: &Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create a task manager with the current context
        let task_manager = TaskManager::with_storage(context)?;

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
    use crate::task::{Task, TaskStatus};
    use crate::task_storage::get_task_storage;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_executor_lifecycle() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );

        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
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

    #[tokio::test]
    async fn test_executor_completes_task_without_deadlock() {
        // This test verifies that the executor doesn't deadlock after completing a task
        let temp_dir = TempDir::new().unwrap();
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );

        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a mock task that will complete successfully
        let task = Task::new(
            "test-task-123".to_string(),
            temp_dir.path().to_path_buf(),
            "test-task".to_string(),
            "test".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/test-task-123".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
        );

        // Set up storage with the queued task
        let fs = Arc::new(MockFileSystem::new());
        let storage = get_task_storage(xdg.clone(), fs.clone());
        storage.add_task(task.clone()).await.unwrap();

        // Create app context with a mock docker client that always succeeds
        let app_context = crate::context::AppContext::builder()
            .with_file_system(fs)
            .with_xdg_directories(xdg)
            .with_docker_client(Arc::new(crate::test_utils::NoOpDockerClient))
            .build();

        let storage = Arc::new(Mutex::new(storage));
        let executor = TaskExecutor::new(Arc::new(app_context), storage.clone());
        let executor = Arc::new(executor);

        // Start executor in background
        let exec_handle = {
            let exec = executor.clone();
            tokio::spawn(async move {
                let _ = exec.start().await;
            })
        };

        // Wait for the task to be processed
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Check that the task was updated to COMPLETE or FAILED (not stuck in RUNNING)
        let storage_guard = storage.lock().await;
        let updated_task = storage_guard.get_task(&task.id).await.unwrap().unwrap();
        drop(storage_guard);

        // The task should not be stuck in RUNNING state
        assert_ne!(
            updated_task.status,
            TaskStatus::Running,
            "Task should not be stuck in RUNNING state - indicates a deadlock"
        );

        // Add another queued task to verify the executor can continue processing
        let task2 = Task::new(
            "test-task-456".to_string(),
            temp_dir.path().to_path_buf(),
            "test-task-2".to_string(),
            "test".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/test-task-456".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
        );

        let storage_guard = storage.lock().await;
        storage_guard.add_task(task2.clone()).await.unwrap();
        drop(storage_guard);

        // Wait a bit more to see if the executor picks up the second task
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Stop executor
        executor.stop().await;
        let _ = exec_handle.await;

        // Verify the executor was able to process tasks after the first one completed
        let storage_guard = storage.lock().await;
        let all_tasks = storage_guard.list_tasks().await.unwrap();
        drop(storage_guard);

        // At least one task should have been processed
        let processed_tasks = all_tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Queued)
            .count();
        assert!(
            processed_tasks >= 1,
            "Executor should have processed at least one task"
        );
    }

    #[tokio::test]
    async fn test_parallel_execution() {
        // Test that multiple tasks can run in parallel
        let temp_dir = TempDir::new().unwrap();
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );

        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create multiple tasks
        let mut tasks = vec![];
        for i in 0..3 {
            let task = Task::new(
                format!("test-task-{}", i),
                temp_dir.path().to_path_buf(),
                format!("test-task-{}", i),
                "test".to_string(),
                "instructions.md".to_string(),
                "claude-code".to_string(),
                30,
                format!("tsk/test-task-{}", i),
                "abc123".to_string(),
                "default".to_string(),
                "default".to_string(),
                chrono::Local::now(),
                temp_dir.path().to_path_buf(),
            );
            tasks.push(task);
        }

        // Set up storage with queued tasks
        let fs = Arc::new(MockFileSystem::new());
        let storage = get_task_storage(xdg.clone(), fs.clone());
        for task in &tasks {
            storage.add_task(task.clone()).await.unwrap();
        }

        // Create app context
        let app_context = crate::context::AppContext::builder()
            .with_file_system(fs)
            .with_xdg_directories(xdg)
            .with_docker_client(Arc::new(crate::test_utils::NoOpDockerClient))
            .build();

        let storage = Arc::new(Mutex::new(storage));
        // Create executor with 2 workers
        let executor = TaskExecutor::with_workers(Arc::new(app_context), storage.clone(), 2);
        let executor = Arc::new(executor);

        // Start executor in background
        let exec_handle = {
            let exec = executor.clone();
            tokio::spawn(async move {
                let _ = exec.start().await;
            })
        };

        // Give some time for parallel execution
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Stop executor
        executor.stop().await;
        let _ = exec_handle.await;

        // Check that multiple tasks were processed
        let storage_guard = storage.lock().await;
        let all_tasks = storage_guard.list_tasks().await.unwrap();
        drop(storage_guard);

        let processed_tasks = all_tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Queued)
            .count();

        // With 2 workers and 3 tasks, at least 2 should be processed
        assert!(
            processed_tasks >= 2,
            "With 2 workers, at least 2 tasks should have been processed, but only {} were",
            processed_tasks
        );
    }
}
