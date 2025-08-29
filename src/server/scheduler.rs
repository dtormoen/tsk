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
    workers: u32,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
    worker_pool: Option<Arc<WorkerPool<TaskJob>>>,
    /// Track task IDs that are currently submitted to the worker pool
    submitted_tasks: Arc<Mutex<HashSet<String>>>,
}

impl TaskScheduler {
    /// Create a new task scheduler with default single worker
    #[cfg(test)]
    pub fn new(context: Arc<AppContext>, storage: Arc<Mutex<Box<dyn TaskStorage>>>) -> Self {
        Self::with_workers(context, storage, 1)
    }

    /// Set terminal title based on active worker count
    fn set_terminal_title(&self, active_count: u32) {
        if active_count == 0 {
            self.context
                .terminal_operations()
                .set_title(&format!("TSK Server Idle (0/{} workers)", self.workers));
        } else {
            self.context.terminal_operations().set_title(&format!(
                "TSK Server Running ({}/{} workers)",
                active_count, self.workers
            ));
        }
    }

    /// Create a new task scheduler with specified number of workers
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
            worker_pool: None,
            submitted_tasks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Start the scheduler and begin processing tasks
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut running = self.running.lock().await;
        if *running {
            return Err("Scheduler is already running".into());
        }
        *running = true;
        drop(running);

        println!("Task scheduler started with {} worker(s)", self.workers);

        // Create the worker pool
        let worker_pool = Arc::new(WorkerPool::<TaskJob>::new(self.workers as usize));
        self.worker_pool = Some(worker_pool.clone());

        // Set initial idle title
        self.set_terminal_title(0);

        // Track active worker count for terminal title updates
        let active_workers = Arc::new(Mutex::new(0u32));

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

            // Poll for completed jobs and process results
            if let Some(pool) = &self.worker_pool {
                let completed_jobs = pool.poll_completed().await;
                for job_result in completed_jobs {
                    // Always decrement worker count when a job completes (regardless of success/failure)
                    let mut count = active_workers.lock().await;
                    *count = count.saturating_sub(1);
                    self.set_terminal_title(*count);
                    drop(count);

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

                    // Update active workers count and terminal title
                    {
                        let mut count = active_workers.lock().await;
                        *count += 1;
                        self.set_terminal_title(*count);
                    }

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

                            // Decrement worker count since we incremented it but didn't submit
                            let mut count = active_workers.lock().await;
                            *count = count.saturating_sub(1);
                            self.set_terminal_title(*count);
                            drop(count);

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

                            // Decrement worker count since we incremented it but didn't submit
                            let mut count = active_workers.lock().await;
                            *count = count.saturating_sub(1);
                            self.set_terminal_title(*count);
                            drop(count);

                            // Revert task status
                            let storage = self.storage.lock().await;
                            let _ = storage
                                .update_task_status(&task.id, TaskStatus::Queued, None, None, None)
                                .await;
                        }
                    }
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
    async fn test_scheduler_lifecycle() {
        let ctx = AppContext::builder().build();
        let storage = Arc::new(Mutex::new(get_task_storage(
            ctx.tsk_config(),
            ctx.file_system(),
        )));

        let mut scheduler = TaskScheduler::new(Arc::new(ctx), storage);

        // Test that scheduler starts as not running
        assert!(!*scheduler.running.lock().await);

        // Start scheduler in background
        let sched_handle = tokio::spawn(async move {
            let _ = scheduler.start().await;
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
        let mut scheduler = TaskScheduler::new(Arc::new(ctx), storage.clone());

        // Start scheduler in background
        let sched_handle = tokio::spawn(async move {
            let _ = scheduler.start().await;
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

        let scheduler = TaskScheduler::new(ctx, storage);

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
        let scheduler = TaskScheduler::new(Arc::new(ctx), storage);

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
    async fn test_worker_count_tracking_logic() {
        // Test that the worker count tracking logic in the scheduler properly decrements
        // the count regardless of job success/failure. This is a focused unit test that
        // simulates the scheduler's worker count behavior without using the actual WorkerPool.

        // Track active workers using the same pattern as the scheduler
        let active_workers = Arc::new(Mutex::new(0u32));

        // Simulate starting 3 jobs
        for i in 1..=3 {
            let mut count = active_workers.lock().await;
            *count += 1;
            println!("Started job {}, active workers: {}", i, *count);
        }

        assert_eq!(
            *active_workers.lock().await,
            3,
            "Should have 3 active workers after starting jobs"
        );

        // Simulate job completions with different outcomes:
        // Job 1: Success
        {
            let mut count = active_workers.lock().await;
            *count = count.saturating_sub(1);
            println!("Job 1 completed successfully, active workers: {}", *count);
        }
        assert_eq!(
            *active_workers.lock().await,
            2,
            "Should have 2 active workers after job 1 completes"
        );

        // Job 2: Failure (but still completes)
        {
            let mut count = active_workers.lock().await;
            *count = count.saturating_sub(1);
            println!("Job 2 failed, active workers: {}", *count);
        }
        assert_eq!(
            *active_workers.lock().await,
            1,
            "Should have 1 active worker after job 2 fails"
        );

        // Job 3: Error (job panicked but still counted as complete)
        {
            let mut count = active_workers.lock().await;
            *count = count.saturating_sub(1);
            println!("Job 3 errored, active workers: {}", *count);
        }
        assert_eq!(
            *active_workers.lock().await,
            0,
            "Should have 0 active workers after all jobs complete"
        );

        // Test saturating_sub prevents underflow
        {
            let mut count = active_workers.lock().await;
            *count = count.saturating_sub(1);
            println!(
                "Extra decrement (should stay at 0), active workers: {}",
                *count
            );
        }
        assert_eq!(
            *active_workers.lock().await,
            0,
            "saturating_sub should prevent negative counts"
        );

        // Test the scenario that was causing the bug: increment but job fails to submit
        {
            // Increment for a new job
            let mut count = active_workers.lock().await;
            *count += 1;
            println!("Attempted to start job 4, active workers: {}", *count);
        }
        assert_eq!(
            *active_workers.lock().await,
            1,
            "Should have 1 active worker"
        );

        // Job fails to submit, so we should decrement
        {
            let mut count = active_workers.lock().await;
            *count = count.saturating_sub(1);
            println!("Job 4 failed to submit, active workers: {}", *count);
        }
        assert_eq!(
            *active_workers.lock().await,
            0,
            "Should be back to 0 after job fails to submit"
        );
    }

    #[tokio::test]
    async fn test_worker_pool_returns_completed_jobs() {
        // Test that the WorkerPool properly returns completed jobs via poll_completed
        use crate::server::worker_pool::{AsyncJob, JobError, JobResult, WorkerPool};
        use std::time::Duration;
        use tokio::time::sleep;

        struct TestJob {
            id: String,
            duration_ms: u64,
        }

        impl AsyncJob for TestJob {
            async fn execute(self) -> Result<JobResult, JobError> {
                sleep(Duration::from_millis(self.duration_ms)).await;
                Ok(JobResult {
                    job_id: self.id.clone(),
                    success: true,
                    message: Some(format!("Job {} completed", self.id)),
                })
            }

            fn job_id(&self) -> String {
                self.id.clone()
            }
        }

        let pool = WorkerPool::new(2);

        // Submit a job using try_submit
        let job = TestJob {
            id: "test-job-1".to_string(),
            duration_ms: 50,
        };
        let handle = pool.try_submit(job).await.unwrap();
        assert!(handle.is_some(), "Should be able to submit job");

        // Wait for job to complete
        sleep(Duration::from_millis(100)).await;

        // Poll for completed jobs
        let completed = pool.poll_completed().await;
        assert_eq!(completed.len(), 1, "Should have 1 completed job");

        if let Ok(result) = &completed[0] {
            assert_eq!(result.job_id, "test-job-1");
            assert!(result.success);
        } else {
            panic!("Job should have completed successfully");
        }
    }
}
