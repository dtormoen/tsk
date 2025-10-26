use crate::context::AppContext;
use crate::server::worker_pool::{AsyncJob, JobError, JobResult, WorkerPool};
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_runner::TaskExecutionError;
use crate::task_storage::TaskStorage;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

/// Task scheduler that manages task lifecycle and scheduling decisions
///
/// The scheduler is responsible for:
/// - Selecting tasks from storage for execution
/// - Managing task status transitions
/// - Handling warmup failure wait periods
/// - Delegating actual execution to the worker pool
/// - Preventing double-scheduling of tasks
pub struct TaskScheduler {
    context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    running: Arc<Mutex<bool>>,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
    worker_pool: Option<Arc<WorkerPool<TaskJob>>>,
    /// Track task IDs that are currently submitted to the worker pool
    submitted_tasks: Arc<Mutex<HashSet<String>>>,
    quit_signal: Arc<tokio::sync::Notify>,
    quit_when_done: bool,
}

impl TaskScheduler {
    /// Update terminal title based on worker pool status
    fn update_terminal_title(&self) {
        if let Some(pool) = &self.worker_pool {
            let active = pool.active_workers();
            let total = pool.total_workers();

            if active == 0 {
                self.context
                    .terminal_operations()
                    .set_title(&format!("TSK Server Idle (0/{} workers)", total));
            } else {
                self.context.terminal_operations().set_title(&format!(
                    "TSK Server Running ({}/{} workers)",
                    active, total
                ));
            }
        }
    }

    /// Create a new task scheduler
    pub fn new(
        context: Arc<AppContext>,
        storage: Arc<Mutex<Box<dyn TaskStorage>>>,
        quit_when_done: bool,
        quit_signal: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            context,
            storage,
            running: Arc::new(Mutex::new(false)),
            warmup_failure_wait_until: Arc::new(Mutex::new(None)),
            worker_pool: None,
            submitted_tasks: Arc::new(Mutex::new(HashSet::new())),
            quit_signal,
            quit_when_done,
        }
    }

    /// Start the scheduler and begin processing tasks
    pub async fn start(
        &mut self,
        workers: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut running = self.running.lock().await;
        if *running {
            return Err("Scheduler is already running".into());
        }
        *running = true;
        drop(running);

        println!("Task scheduler started with {} worker(s)", workers);

        if self.quit_when_done {
            println!("Running in quit-when-done mode - will exit when queue is empty");
        }

        // Create the worker pool
        let worker_pool = Arc::new(WorkerPool::<TaskJob>::new(workers as usize));
        self.worker_pool = Some(worker_pool.clone());

        // Set initial idle title
        self.update_terminal_title();

        // Check if we should quit immediately due to empty queue
        if self.quit_when_done {
            let storage = self.storage.lock().await;
            let tasks = storage.list_tasks().await?;
            drop(storage);

            let queued_count = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Queued)
                .count();

            if queued_count == 0 {
                println!("Queue is empty at startup. Exiting immediately...");
                self.quit_signal.notify_one();
                self.stop().await;
                // Loop will check running flag and exit immediately
            }
        }

        // Main scheduling loop
        loop {
            // Check if we should continue running
            if !*self.running.lock().await {
                println!("Task scheduler stopping, waiting for active tasks to complete...");

                // Shutdown the worker pool and wait for all tasks
                if let Some(pool) = &self.worker_pool {
                    let _ = pool.shutdown().await;
                }

                self.context.terminal_operations().restore_title();
                break;
            }

            // Update terminal title to reflect current worker state
            self.update_terminal_title();

            // Poll for completed jobs and process results
            if let Some(pool) = &self.worker_pool {
                let completed_jobs = pool.poll_completed().await;
                for job_result in completed_jobs {
                    match job_result {
                        Ok(result) => {
                            // Remove from submitted tasks
                            self.submitted_tasks.lock().await.remove(&result.job_id);

                            if result.success {
                                println!("Task completed successfully: {}", result.job_id);
                            } else if let Some(msg) = result.message {
                                eprintln!("Task failed: {} - {}", result.job_id, msg);
                            }
                        }
                        Err(e) => {
                            eprintln!("Job error: {}", e);
                        }
                    }
                }
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

            // Try to schedule a new task if workers are available
            if let Some(pool) = &self.worker_pool
                && pool.available_workers() > 0
            {
                // Look for a queued task that isn't already submitted
                let storage = self.storage.lock().await;
                let tasks = storage.list_tasks().await?;
                drop(storage);

                let submitted = self.submitted_tasks.lock().await;
                let queued_task = tasks
                    .into_iter()
                    .find(|t| t.status == TaskStatus::Queued && !submitted.contains(&t.id));
                drop(submitted);

                if let Some(task) = queued_task {
                    println!("Scheduling task: {} ({})", task.name, task.id);

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

                    // Mark task as submitted
                    self.submitted_tasks
                        .lock()
                        .await
                        .insert(running_task.id.clone());

                    // Create and submit the job
                    let job = TaskJob {
                        task: running_task,
                        context: self.context.clone(),
                        storage: self.storage.clone(),
                        warmup_failure_wait_until: self.warmup_failure_wait_until.clone(),
                    };

                    match pool.try_submit(job).await {
                        Ok(Some(_handle)) => {
                            // Job submitted successfully
                        }
                        Ok(None) => {
                            // No workers available (shouldn't happen since we checked)
                            eprintln!("Failed to submit job: no workers available");
                            // Remove from submitted tasks since it didn't actually submit
                            self.submitted_tasks.lock().await.remove(&task.id);

                            // Revert task status
                            let storage = self.storage.lock().await;
                            let _ = storage
                                .update_task_status(&task.id, TaskStatus::Queued, None, None, None)
                                .await;
                        }
                        Err(e) => {
                            eprintln!("Failed to submit job: {}", e);
                            // Remove from submitted tasks since it didn't actually submit
                            self.submitted_tasks.lock().await.remove(&task.id);

                            // Revert task status
                            let storage = self.storage.lock().await;
                            let _ = storage
                                .update_task_status(&task.id, TaskStatus::Queued, None, None, None)
                                .await;
                        }
                    }
                }
            }

            // Check if we should quit when done
            if self.quit_when_done {
                // Get current task list
                let storage = self.storage.lock().await;
                let tasks = storage.list_tasks().await?;
                drop(storage);

                // Count queued tasks
                let queued_count = tasks
                    .iter()
                    .filter(|t| t.status == TaskStatus::Queued)
                    .count();

                // Check: no queued tasks AND no active workers AND not in warmup wait
                let active_workers = if let Some(pool) = &self.worker_pool {
                    pool.active_workers()
                } else {
                    0
                };

                let in_warmup_wait = self.warmup_failure_wait_until.lock().await.is_some();

                if queued_count == 0 && active_workers == 0 && !in_warmup_wait {
                    println!("Queue is empty and no workers active. Shutting down...");
                    self.quit_signal.notify_one();
                    self.stop().await;
                    // Loop will exit on next iteration when running becomes false
                }
            }

            // Sleep briefly before next scheduling check
            sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }

    /// Stop the scheduler
    pub async fn stop(&self) {
        *self.running.lock().await = false;
    }
}

/// Job implementation for executing tasks
pub struct TaskJob {
    task: Task,
    context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
}

impl AsyncJob for TaskJob {
    async fn execute(self) -> Result<JobResult, JobError> {
        // Execute the task using TaskManager
        let result = Self::execute_single_task(&self.context, &self.task).await;

        match result {
            Ok(_) => {
                // Task completed successfully
                Ok(JobResult {
                    job_id: self.task.id.clone(),
                    success: true,
                    message: Some(format!("Task {} completed successfully", self.task.name)),
                })
            }
            Err(e) => {
                // Check if this was a warmup failure
                if e.is_warmup_failure {
                    println!(
                        "Task {} failed during warmup. Setting 1-hour wait period...",
                        self.task.id
                    );

                    // Set the wait period
                    let wait_until = Instant::now() + Duration::from_secs(3600); // 1 hour
                    *self.warmup_failure_wait_until.lock().await = Some(wait_until);

                    // Reset task status to QUEUED so it can be retried
                    let storage_lock = self.storage.lock().await;
                    if let Err(e) = storage_lock
                        .update_task_status(
                            &self.task.id,
                            TaskStatus::Queued,
                            None,
                            None,
                            Some("Warmup failed, will retry after wait period".to_string()),
                        )
                        .await
                    {
                        eprintln!("Failed to reset task status to QUEUED: {e}");
                    }
                }

                Ok(JobResult {
                    job_id: self.task.id.clone(),
                    success: false,
                    message: Some(e.message),
                })
            }
        }
    }

    fn job_id(&self) -> String {
        self.task.id.clone()
    }
}

impl TaskJob {
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
    use crate::task::{Task, TaskStatus};
    use crate::task_storage::get_task_storage;
    use crate::test_utils::git_test_utils::TestGitRepository;

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
            "claude".to_string(),
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
    async fn test_scheduler_lifecycle() {
        let ctx = AppContext::builder().build();
        let storage = Arc::new(Mutex::new(get_task_storage(
            ctx.tsk_config(),
            ctx.file_system(),
        )));

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let mut scheduler = TaskScheduler::new(Arc::new(ctx), storage, false, quit_signal);

        // Test that scheduler starts as not running
        assert!(!*scheduler.running.lock().await);

        // Start scheduler in background with 1 worker
        let sched_handle = tokio::spawn(async move {
            let _ = scheduler.start(1).await;
        });

        // Give the scheduler time to start
        sleep(Duration::from_millis(100)).await;

        // Stop scheduler by aborting the task
        sched_handle.abort();
        let _ = sched_handle.await;
    }

    #[tokio::test]
    async fn test_scheduler_processes_tasks() {
        // Test that scheduler processes tasks without deadlock and can handle multiple tasks
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();

        let (test_repo, commit_sha) = setup_test_repo().unwrap();

        // Create two tasks
        let task1 = create_test_task("task-1", test_repo.path(), &commit_sha, config.data_dir());
        let task2 = create_test_task("task-2", test_repo.path(), &commit_sha, config.data_dir());

        // Set up storage with the first task
        let storage = get_task_storage(config, ctx.file_system());
        storage.add_task(task1.clone()).await.unwrap();

        let storage = Arc::new(Mutex::new(storage));
        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let mut scheduler = TaskScheduler::new(Arc::new(ctx), storage.clone(), false, quit_signal);

        // Start scheduler in background with 1 worker
        let sched_handle = tokio::spawn(async move {
            let _ = scheduler.start(1).await;
        });

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

        // Add second task to verify scheduler continues processing
        {
            let storage_guard = storage.lock().await;
            storage_guard.add_task(task2.clone()).await.unwrap();
        }

        // Wait for second task to start processing (shows scheduler isn't deadlocked)
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

        // Stop scheduler by dropping handle (task will abort)
        sched_handle.abort();
        let _ = sched_handle.await;
    }

    #[tokio::test]
    async fn test_warmup_failure_wait_behavior() {
        // Test that the scheduler properly handles warmup failure wait periods
        let ctx = Arc::new(AppContext::builder().build());

        let storage = Arc::new(Mutex::new(get_task_storage(
            ctx.tsk_config(),
            ctx.file_system(),
        )));

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let scheduler = TaskScheduler::new(ctx, storage, false, quit_signal);

        // Initially no wait period
        assert!(
            scheduler.warmup_failure_wait_until.lock().await.is_none(),
            "Should start with no wait period"
        );

        // Set a wait period in the future
        let future_time = Instant::now() + Duration::from_secs(3600);
        *scheduler.warmup_failure_wait_until.lock().await = Some(future_time);

        // Verify wait period is set and in the future
        let wait_until = *scheduler.warmup_failure_wait_until.lock().await;
        assert!(wait_until.is_some(), "Wait period should be set");
        assert!(
            wait_until.unwrap() > Instant::now(),
            "Wait period should be in the future"
        );

        // Clear the wait period
        *scheduler.warmup_failure_wait_until.lock().await = None;

        // Verify it's cleared
        assert!(
            scheduler.warmup_failure_wait_until.lock().await.is_none(),
            "Wait period should be cleared"
        );
    }

    #[tokio::test]
    async fn test_scheduler_prevents_double_scheduling() {
        // Test that the scheduler doesn't schedule the same task twice
        let ctx = AppContext::builder().build();
        let storage = Arc::new(Mutex::new(get_task_storage(
            ctx.tsk_config(),
            ctx.file_system(),
        )));
        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let scheduler = TaskScheduler::new(Arc::new(ctx), storage, false, quit_signal);

        // Add a task ID to submitted tasks
        scheduler
            .submitted_tasks
            .lock()
            .await
            .insert("task-1".to_string());

        // Verify it's tracked
        assert!(scheduler.submitted_tasks.lock().await.contains("task-1"));

        // Remove it
        scheduler.submitted_tasks.lock().await.remove("task-1");

        // Verify it's removed
        assert!(!scheduler.submitted_tasks.lock().await.contains("task-1"));
    }

    #[tokio::test]
    async fn test_worker_pool_count_tracking() {
        // Test that the worker pool properly tracks active and available workers
        use crate::server::worker_pool::{AsyncJob, JobError, JobResult, WorkerPool};

        // Simple test job for worker pool testing
        struct SimpleJob {
            id: String,
            should_succeed: bool,
        }

        impl AsyncJob for SimpleJob {
            async fn execute(self) -> Result<JobResult, JobError> {
                sleep(Duration::from_millis(10)).await;
                if self.should_succeed {
                    Ok(JobResult {
                        job_id: self.id,
                        success: true,
                        message: Some("Success".to_string()),
                    })
                } else {
                    Ok(JobResult {
                        job_id: self.id,
                        success: false,
                        message: Some("Failed".to_string()),
                    })
                }
            }

            fn job_id(&self) -> String {
                self.id.clone()
            }
        }

        // Create a worker pool with 3 workers
        let pool = WorkerPool::new(3);

        // Initially all workers should be available
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 3);
        assert_eq!(pool.active_workers(), 0);

        // Submit 2 jobs
        let job1 = SimpleJob {
            id: "job-1".to_string(),
            should_succeed: true,
        };
        let job2 = SimpleJob {
            id: "job-2".to_string(),
            should_succeed: false,
        };

        pool.try_submit(job1).await.unwrap().unwrap();
        pool.try_submit(job2).await.unwrap().unwrap();

        // Give jobs a moment to start
        sleep(Duration::from_millis(5)).await;

        // Check counts with 2 active jobs
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 1);
        assert_eq!(pool.active_workers(), 2);

        // Wait for jobs to complete
        sleep(Duration::from_millis(20)).await;

        // Poll completed jobs (this releases the permits)
        let completed = pool.poll_completed().await;
        assert_eq!(completed.len(), 2);

        // All workers should be available again
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 3);
        assert_eq!(pool.active_workers(), 0);
    }
}
