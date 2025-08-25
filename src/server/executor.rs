use crate::context::AppContext;
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_runner::TaskExecutionError;
use crate::task_storage::TaskStorage;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant, sleep};

/// Task executor that runs tasks in parallel with configurable workers
pub struct TaskExecutor {
    context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    running: Arc<Mutex<bool>>,
    workers: u32,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
}

impl TaskExecutor {
    /// Create a new task executor with default single worker
    #[cfg(test)]
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
            warmup_failure_wait_until: Arc::new(Mutex::new(None)),
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

        // Track active worker count
        let active_workers = Arc::new(Mutex::new(0u32));

        loop {
            // Check if we should continue running
            if !*self.running.lock().await {
                println!("Task executor stopping, waiting for active tasks to complete...");

                // Wait for all active tasks to complete
                while active_tasks.join_next().await.is_some() {}

                self.context.terminal_operations().restore_title();
                break;
            }

            // Check if we're in a warmup failure wait period
            let wait_until = self.warmup_failure_wait_until.lock().await;
            if let Some(wait_instant) = *wait_until {
                if Instant::now() < wait_instant {
                    let remaining = wait_instant - Instant::now();
                    drop(wait_until);
                    println!(
                        "Waiting {} seconds due to warmup failure before attempting new tasks...",
                        remaining.as_secs()
                    );
                    // Sleep for the minimum of remaining time or 60 seconds
                    let sleep_duration = std::cmp::min(remaining, Duration::from_secs(60));
                    sleep(sleep_duration).await;
                    continue;
                }
                drop(wait_until);
                // Clear the wait period
                *self.warmup_failure_wait_until.lock().await = None;
                println!("Warmup failure wait period has ended, resuming task processing");
            } else {
                drop(wait_until);
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

            let permit = permit.unwrap();

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

                    // Update active workers count and terminal title
                    {
                        let mut count = active_workers.lock().await;
                        *count += 1;
                        self.context.terminal_operations().set_title(&format!(
                            "TSK Server Running ({}/{} workers)",
                            *count, self.workers
                        ));
                    }

                    // Spawn task execution
                    let context = self.context.clone();
                    let active_workers = active_workers.clone();
                    let terminal_ops = self.context.terminal_operations();
                    let total_workers = self.workers;
                    let warmup_failure_wait_until = self.warmup_failure_wait_until.clone();
                    let storage = self.storage.clone();

                    active_tasks.spawn(async move {
                        // Hold the permit for the entire duration of task execution
                        let _permit = permit;

                        // Execute the task
                        let execution_result =
                            Self::execute_single_task(&context, &running_task).await;

                        match execution_result {
                            Ok(_) => {
                                println!("Task completed successfully: {}", running_task.id);
                                // Task status is already updated by TaskManager.execute_queued_task()
                            }
                            Err(e) => {
                                eprintln!("Task failed: {} - {}", running_task.id, e.message);
                                // Task status is already updated by TaskManager.execute_queued_task()

                                // Check if this was a warmup failure
                                if e.is_warmup_failure {
                                    println!("Task {} failed during warmup. Setting 1-hour wait period...", running_task.id);

                                    // Set the wait period
                                    let wait_until = Instant::now() + Duration::from_secs(3600); // 1 hour
                                    *warmup_failure_wait_until.lock().await = Some(wait_until);

                                    // Reset task status to QUEUED so it can be retried
                                    let storage_lock = storage.lock().await;
                                    if let Err(e) = storage_lock
                                        .update_task_status(
                                            &running_task.id,
                                            TaskStatus::Queued,
                                            None,
                                            None,
                                            Some("Warmup failed, will retry after wait period".to_string()),
                                        )
                                        .await {
                                        eprintln!("Failed to reset task status to QUEUED: {e}");
                                    }
                                }
                            }
                        }

                        // Update active workers count and terminal title
                        let mut count = active_workers.lock().await;
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            terminal_ops
                                .set_title(&format!("TSK Server Idle (0/{total_workers} workers)"));
                        } else {
                            terminal_ops.set_title(&format!(
                                "TSK Server Running ({}/{} workers)",
                                *count, total_workers
                            ));
                        }

                        // Permit is automatically dropped here, releasing the semaphore
                    });
                }
                None => {
                    // No tasks to execute, release the permit and wait
                    drop(permit);
                    sleep(Duration::from_secs(1)).await;
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
    ) -> Result<(), TaskExecutionError> {
        // Create a task manager with the current context
        let task_manager = TaskManager::new(context).map_err(|e| TaskExecutionError {
            message: e,
            is_warmup_failure: false,
        })?;

        // Execute the task
        let result = task_manager.execute_queued_task(task).await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::DefaultFileSystem;
    use crate::task::{Task, TaskStatus};
    use crate::task_storage::get_task_storage;
    use crate::test_utils::git_test_utils::TestGitRepository;
    use tokio::time::timeout;

    /// Wait for tasks to reach a certain state with timeout.
    ///
    /// Polls the storage periodically to check if the condition is met.
    /// Returns true if condition met, false if timeout reached.
    async fn wait_for_condition<F>(
        storage: &Arc<Mutex<Box<dyn TaskStorage>>>,
        timeout_duration: Duration,
        mut condition: F,
    ) -> bool
    where
        F: FnMut(&[Task]) -> bool,
    {
        let deadline = Instant::now() + timeout_duration;

        while Instant::now() < deadline {
            let storage_guard = storage.lock().await;
            let tasks = storage_guard.list_tasks().await.unwrap();
            drop(storage_guard);

            if condition(&tasks) {
                return true;
            }

            // Small delay to avoid busy waiting
            sleep(Duration::from_millis(50)).await;
        }

        false
    }

    /// Create a standard test task with given ID.
    fn create_test_task(
        id: &str,
        repo_path: &std::path::Path,
        commit_sha: &str,
        data_dir: &std::path::Path,
    ) -> Task {
        Task::new(
            id.to_string(),
            repo_path.to_path_buf(),
            format!("task-{id}"),
            "test".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            format!("tsk/test/{id}"),
            commit_sha.to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            data_dir.join(format!("task-copy-{id}")),
            false,
        )
    }

    /// Setup test repository with instructions file.
    fn setup_test_repo() -> Result<(TestGitRepository, String), Box<dyn std::error::Error>> {
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        test_repo.create_file("instructions.md", "Test instructions")?;
        test_repo.stage_all()?;
        let commit_sha = test_repo.commit("Add instructions")?;
        Ok((test_repo, commit_sha))
    }

    #[tokio::test]
    async fn test_executor_lifecycle() {
        let app_context = AppContext::builder().build();
        let config = app_context.tsk_config();
        config.ensure_directories().unwrap();

        let fs = Arc::new(DefaultFileSystem);
        let storage = Arc::new(Mutex::new(get_task_storage(config, fs)));

        let executor = TaskExecutor::new(Arc::new(app_context), storage);
        let executor = Arc::new(executor);

        // Test that executor starts as not running
        assert!(!*executor.running.lock().await);

        // Start executor in background
        let exec_handle = {
            let exec = executor.clone();
            tokio::spawn(async move {
                let _ = exec.start().await;
            })
        };

        // Wait for executor to start (poll the running flag)
        let started = timeout(Duration::from_secs(1), async {
            while !*executor.running.lock().await {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok();

        assert!(started, "Executor should have started within timeout");
        assert!(*executor.running.lock().await);

        // Stop executor
        executor.stop().await;
        let _ = exec_handle.await;

        assert!(!*executor.running.lock().await);
    }

    #[tokio::test]
    async fn test_executor_processes_tasks() {
        // Test that executor processes tasks without deadlock and can handle multiple tasks
        let app_context = AppContext::builder().build();
        let config = app_context.tsk_config();
        config.ensure_directories().unwrap();

        let (test_repo, commit_sha) = setup_test_repo().unwrap();

        // Create two tasks
        let task1 = create_test_task("task-1", test_repo.path(), &commit_sha, config.data_dir());
        let task2 = create_test_task("task-2", test_repo.path(), &commit_sha, config.data_dir());

        // Set up storage with the first task
        let fs = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(config, fs);
        storage.add_task(task1.clone()).await.unwrap();

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

        // Wait for first task to complete (not be stuck in RUNNING)
        let task1_processed = wait_for_condition(&storage, Duration::from_secs(5), |tasks| {
            tasks
                .iter()
                .find(|t| t.id == task1.id)
                .map(|t| t.status != TaskStatus::Queued && t.status != TaskStatus::Running)
                .unwrap_or(false)
        })
        .await;

        assert!(
            task1_processed,
            "First task should be processed within timeout"
        );

        // Add second task to verify executor continues processing
        {
            let storage_guard = storage.lock().await;
            storage_guard.add_task(task2.clone()).await.unwrap();
        }

        // Wait for second task to start processing (shows executor isn't deadlocked)
        let task2_started = wait_for_condition(&storage, Duration::from_secs(5), |tasks| {
            tasks
                .iter()
                .find(|t| t.id == task2.id)
                .map(|t| t.status != TaskStatus::Queued)
                .unwrap_or(false)
        })
        .await;

        assert!(
            task2_started,
            "Second task should start processing, indicating no deadlock"
        );

        // Stop executor
        executor.stop().await;
        let _ = exec_handle.await;
    }

    #[tokio::test]
    async fn test_warmup_failure_wait_behavior() {
        // Test that the executor properly handles warmup failure wait periods
        let app_context = Arc::new(AppContext::builder().build());
        let xdg = app_context.tsk_config();
        xdg.ensure_directories().unwrap();

        let fs = Arc::new(DefaultFileSystem);
        let storage = Arc::new(Mutex::new(get_task_storage(xdg, fs)));

        let executor = TaskExecutor::new(app_context, storage);

        // Initially no wait period
        assert!(
            executor.warmup_failure_wait_until.lock().await.is_none(),
            "Should start with no wait period"
        );

        // Set a wait period in the future
        let future_time = Instant::now() + Duration::from_secs(3600);
        *executor.warmup_failure_wait_until.lock().await = Some(future_time);

        // Verify wait period is set and in the future
        let wait_until = *executor.warmup_failure_wait_until.lock().await;
        assert!(wait_until.is_some(), "Wait period should be set");
        assert!(
            wait_until.unwrap() > Instant::now(),
            "Wait period should be in the future"
        );

        // Clear the wait period
        *executor.warmup_failure_wait_until.lock().await = None;

        // Verify it's cleared
        assert!(
            executor.warmup_failure_wait_until.lock().await.is_none(),
            "Wait period should be cleared"
        );
    }
}
