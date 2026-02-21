use crate::context::AppContext;
use crate::context::docker_client::DockerClient;
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::git_operations;
use crate::server::worker_pool::{AsyncJob, JobError, JobResult, WorkerPool};
use crate::task::{Task, TaskStatus};
use crate::task_manager::TaskManager;
use crate::task_runner::{TaskExecutionError, TaskRunner};
use crate::task_storage::TaskStorage;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

/// Represents the readiness status of a task's parent
#[derive(Debug)]
enum ParentStatus {
    /// Parent task is complete, includes the parent task for repo preparation
    Ready(Box<Task>),
    /// Parent task is still queued or running
    Waiting,
    /// Parent task failed
    Failed(String),
    /// Parent task was not found in storage
    NotFound(String),
}

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
    docker_client: Arc<dyn DockerClient>,
    storage: Arc<dyn TaskStorage>,
    running: Arc<Mutex<bool>>,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
    worker_pool: Option<Arc<WorkerPool<TaskJob>>>,
    /// Track task IDs that are currently submitted to the worker pool
    submitted_tasks: Arc<Mutex<HashSet<String>>>,
    quit_signal: Arc<tokio::sync::Notify>,
    quit_when_done: bool,
    last_auto_clean: Instant,
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
        docker_client: Arc<dyn DockerClient>,
        storage: Arc<dyn TaskStorage>,
        quit_when_done: bool,
        quit_signal: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            context,
            docker_client,
            storage,
            running: Arc::new(Mutex::new(false)),
            warmup_failure_wait_until: Arc::new(Mutex::new(None)),
            worker_pool: None,
            submitted_tasks: Arc::new(Mutex::new(HashSet::new())),
            quit_signal,
            quit_when_done,
            last_auto_clean: Instant::now() - Duration::from_secs(3600),
        }
    }

    /// Check if a task's parent is ready (complete or non-existent).
    ///
    /// Returns:
    /// - `None` if the task has no parent
    /// - `Some(Ready(parent_task))` if parent is complete
    /// - `Some(Waiting)` if parent is still queued or running
    /// - `Some(Failed(msg))` if parent failed
    /// - `Some(NotFound(id))` if parent doesn't exist in storage
    fn is_parent_ready(task: &Task, all_tasks: &[Task]) -> Option<ParentStatus> {
        let parent_id = task.parent_ids.first()?;

        // Find the parent task
        let parent_task = all_tasks.iter().find(|t| &t.id == parent_id);

        match parent_task {
            None => Some(ParentStatus::NotFound(parent_id.clone())),
            Some(parent) => match parent.status {
                TaskStatus::Complete => Some(ParentStatus::Ready(Box::new(parent.clone()))),
                TaskStatus::Failed => Some(ParentStatus::Failed(format!(
                    "Parent task {} failed",
                    parent_id
                ))),
                TaskStatus::Queued | TaskStatus::Running => Some(ParentStatus::Waiting),
            },
        }
    }

    /// Prepare a child task for scheduling by copying the repository from the parent task.
    ///
    /// This updates the task's `copied_repo_path`, `source_commit`, and `source_branch` fields.
    /// Returns the updated task, or an error if preparation fails.
    async fn prepare_child_task(&self, task: &Task, parent_task: &Task) -> Result<Task, String> {
        // Get the parent task's copied repo path
        let parent_repo_path = parent_task
            .copied_repo_path
            .as_ref()
            .ok_or_else(|| format!("Parent task {} has no copied_repo_path", parent_task.id))?;

        // Get the HEAD commit from the parent's repo
        let source_commit: String = git_operations::get_current_commit(parent_repo_path)
            .await
            .map_err(|e| format!("Failed to get parent HEAD commit: {e}"))?;

        // Copy the repository from the parent task's folder
        let repo_manager = RepoManager::new(&self.context);
        let (copied_repo_path, _branch_name) = repo_manager
            .copy_repo(
                &task.id,
                parent_repo_path,
                Some(&source_commit),
                &task.branch_name,
            )
            .await
            .map_err(|e| format!("Failed to copy repo from parent task: {e}"))?;

        // Create the updated task
        let mut updated_task = task.clone();
        updated_task.copied_repo_path = Some(copied_repo_path);
        updated_task.source_commit = source_commit;
        // Set source_branch to the parent's branch for git-town integration
        updated_task.source_branch = Some(parent_task.branch_name.clone());

        Ok(updated_task)
    }

    /// Mark child tasks as failed when their parent task fails.
    ///
    /// This implements cascading failures: when a task fails, all tasks that
    /// have it as their parent are also marked as failed.
    async fn fail_child_tasks(
        &self,
        failed_task_id: &str,
        all_tasks: &[Task],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Find all tasks that have the failed task as their parent
        let child_tasks: Vec<&Task> = all_tasks
            .iter()
            .filter(|t| t.parent_ids.contains(&failed_task_id.to_string()))
            .collect();

        for task in child_tasks {
            println!(
                "Marking task {} as failed due to parent task failure",
                task.id
            );
            self.storage
                .update_task_status(
                    &task.id,
                    TaskStatus::Failed,
                    None,
                    Some(chrono::Utc::now()),
                    Some(format!("Parent task {} failed", failed_task_id)),
                )
                .await?;
        }

        Ok(())
    }

    /// Check if a task is ready for scheduling.
    ///
    /// A task is ready if:
    /// - It is in Queued status
    /// - It is not already submitted
    /// - It has no parent, OR its parent is complete
    fn is_task_ready_for_scheduling(
        task: &Task,
        all_tasks: &[Task],
        submitted: &HashSet<String>,
    ) -> bool {
        // Must be queued and not already submitted
        if task.status != TaskStatus::Queued || submitted.contains(&task.id) {
            return false;
        }

        // Check parent status
        match Self::is_parent_ready(task, all_tasks) {
            None => true,                             // No parent
            Some(ParentStatus::Ready(_)) => true,     // Parent complete
            Some(ParentStatus::Waiting) => false,     // Still waiting
            Some(ParentStatus::Failed(_)) => false,   // Will be failed by cascade
            Some(ParentStatus::NotFound(_)) => false, // Will be failed by handler
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
            let tasks = self.storage.list_tasks().await?;

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

            // Auto-clean old completed/failed tasks every hour
            const AUTO_CLEAN_INTERVAL: Duration = Duration::from_secs(3600);
            let server_config = &self.context.tsk_config().server;
            if server_config.auto_clean_enabled
                && self.last_auto_clean.elapsed() >= AUTO_CLEAN_INTERVAL
            {
                self.last_auto_clean = Instant::now();
                let min_age = server_config.auto_clean_min_age();
                let task_manager = TaskManager::new(&self.context);
                match task_manager {
                    Ok(tm) => match tm.clean_tasks(true, Some(min_age)).await {
                        Ok(result) if result.deleted > 0 => {
                            println!(
                                "Auto-clean: removed {} old task(s) ({} skipped)",
                                result.deleted, result.skipped
                            );
                        }
                        Err(e) => {
                            eprintln!("Auto-clean failed: {}", e);
                        }
                        _ => {}
                    },
                    Err(e) => {
                        eprintln!("Auto-clean: failed to create task manager: {}", e);
                    }
                }
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
                            } else if let Some(msg) = &result.message {
                                eprintln!("Task failed: {} - {}", result.job_id, msg);
                                // Handle cascading failures for child tasks
                                let tasks = self.storage.list_tasks().await?;
                                self.fail_child_tasks(&result.job_id, &tasks).await?;
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
                // Look for a queued task that isn't already submitted and has a ready parent
                let tasks = self.storage.list_tasks().await?;

                let submitted = self.submitted_tasks.lock().await.clone();

                // First, handle tasks with missing or failed parents
                for task in &tasks {
                    if task.status != TaskStatus::Queued || submitted.contains(&task.id) {
                        continue;
                    }

                    match Self::is_parent_ready(task, &tasks) {
                        Some(ParentStatus::NotFound(pid)) => {
                            // Mark task as failed - parent doesn't exist
                            eprintln!(
                                "Task {} has missing parent {}, marking as failed",
                                task.id, pid
                            );
                            let _ = self
                                .storage
                                .update_task_status(
                                    &task.id,
                                    TaskStatus::Failed,
                                    None,
                                    Some(chrono::Utc::now()),
                                    Some(format!("Parent task {} not found", pid)),
                                )
                                .await;
                        }
                        Some(ParentStatus::Failed(msg)) => {
                            // Mark task as failed - parent failed (cascade)
                            eprintln!("Task {} has failed parent, marking as failed", task.id);
                            let _ = self
                                .storage
                                .update_task_status(
                                    &task.id,
                                    TaskStatus::Failed,
                                    None,
                                    Some(chrono::Utc::now()),
                                    Some(msg),
                                )
                                .await;
                        }
                        _ => {}
                    }
                }

                // Re-fetch tasks after potential status changes
                let tasks = self.storage.list_tasks().await?;

                // Find the first task ready for scheduling
                let queued_task = tasks
                    .iter()
                    .find(|t| Self::is_task_ready_for_scheduling(t, &tasks, &submitted))
                    .cloned();

                if let Some(mut task) = queued_task {
                    // Check if this is a child task that needs repo preparation
                    if !task.parent_ids.is_empty() && task.copied_repo_path.is_none() {
                        // Find the parent task
                        if let Some(ParentStatus::Ready(parent_task)) =
                            Self::is_parent_ready(&task, &tasks)
                        {
                            println!(
                                "Preparing child task {} from parent {}",
                                task.id, parent_task.id
                            );
                            match self.prepare_child_task(&task, &parent_task).await {
                                Ok(prepared_task) => {
                                    // Update the task in storage with the prepared fields
                                    if let Err(e) =
                                        self.storage.update_task(prepared_task.clone()).await
                                    {
                                        eprintln!(
                                            "Failed to update prepared task in storage: {}",
                                            e
                                        );
                                        continue;
                                    }
                                    task = prepared_task;
                                }
                                Err(e) => {
                                    eprintln!("Failed to prepare child task {}: {}", task.id, e);
                                    let _ = self
                                        .storage
                                        .update_task_status(
                                            &task.id,
                                            TaskStatus::Failed,
                                            None,
                                            Some(chrono::Utc::now()),
                                            Some(format!("Failed to prepare repository: {}", e)),
                                        )
                                        .await;
                                    continue;
                                }
                            }
                        }
                    }

                    println!("Scheduling task: {} ({})", task.name, task.id);

                    // Update task status to running
                    let mut running_task = task.clone();
                    running_task.status = TaskStatus::Running;
                    running_task.started_at = Some(chrono::Utc::now());

                    self.storage
                        .update_task_status(
                            &running_task.id,
                            TaskStatus::Running,
                            Some(chrono::Utc::now()),
                            None,
                            None,
                        )
                        .await?;

                    // Mark task as submitted
                    self.submitted_tasks
                        .lock()
                        .await
                        .insert(running_task.id.clone());

                    // Create and submit the job
                    let job = TaskJob {
                        task: running_task,
                        context: self.context.clone(),
                        docker_client: self.docker_client.clone(),
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
                            let _ = self
                                .storage
                                .update_task_status(&task.id, TaskStatus::Queued, None, None, None)
                                .await;
                        }
                        Err(e) => {
                            eprintln!("Failed to submit job: {}", e);
                            // Remove from submitted tasks since it didn't actually submit
                            self.submitted_tasks.lock().await.remove(&task.id);

                            // Revert task status
                            let _ = self
                                .storage
                                .update_task_status(&task.id, TaskStatus::Queued, None, None, None)
                                .await;
                        }
                    }
                }
            }

            // Check if we should quit when done
            if self.quit_when_done {
                // Get current task list
                let tasks = self.storage.list_tasks().await?;

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

    /// Get a handle to the running flag, allowing external code to stop the scheduler
    pub fn running_flag(&self) -> Arc<Mutex<bool>> {
        self.running.clone()
    }

    /// Get a clone of the submitted task IDs Arc for external use
    pub fn submitted_task_ids(&self) -> Arc<Mutex<HashSet<String>>> {
        self.submitted_tasks.clone()
    }
}

/// Job implementation for executing tasks
pub struct TaskJob {
    task: Task,
    context: Arc<AppContext>,
    docker_client: Arc<dyn DockerClient>,
    storage: Arc<dyn TaskStorage>,
    warmup_failure_wait_until: Arc<Mutex<Option<Instant>>>,
}

impl AsyncJob for TaskJob {
    async fn execute(self) -> Result<JobResult, JobError> {
        // Execute the task using TaskManager
        let result =
            Self::execute_single_task(&self.context, self.docker_client.clone(), &self.task).await;

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
                    if let Err(e) = self
                        .storage
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
        docker_client: Arc<dyn DockerClient>,
        task: &Task,
    ) -> Result<(), TaskExecutionError> {
        // Create a task manager with Docker execution capabilities
        let docker_manager = DockerManager::new(context, docker_client);
        let task_runner = TaskRunner::new(context, docker_manager);
        let task_manager =
            TaskManager::with_runner(context, task_runner).map_err(|e| TaskExecutionError {
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
    use crate::test_utils::NoOpDockerClient;
    use crate::test_utils::git_test_utils::TestGitRepository;

    fn test_docker_client() -> Arc<dyn DockerClient> {
        Arc::new(NoOpDockerClient)
    }

    /// Wait for tasks to reach a certain state with timeout.
    ///
    /// Polls the storage periodically to check if the condition is met.
    /// Returns true if condition met, false if timeout reached.
    async fn wait_for_condition<F>(
        storage: &Arc<dyn TaskStorage>,
        timeout_duration: Duration,
        mut condition: F,
    ) -> bool
    where
        F: FnMut(&[Task]) -> bool,
    {
        let deadline = Instant::now() + timeout_duration;

        while Instant::now() < deadline {
            let tasks = storage.list_tasks().await.unwrap();

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
        Task {
            id: id.to_string(),
            repo_root: repo_path.to_path_buf(),
            name: format!("task-{id}"),
            task_type: "test".to_string(),
            branch_name: format!("tsk/test/{id}"),
            source_commit: commit_sha.to_string(),
            copied_repo_path: Some(data_dir.join(format!("task-copy-{id}"))),
            ..Task::test_default()
        }
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
        let storage = get_task_storage(ctx.tsk_env());

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let mut scheduler = TaskScheduler::new(
            Arc::new(ctx),
            test_docker_client(),
            storage,
            false,
            quit_signal,
        );

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
        let tsk_env = ctx.tsk_env();

        let (test_repo, commit_sha) = setup_test_repo().unwrap();

        // Create two tasks
        let task1 = create_test_task("task-1", test_repo.path(), &commit_sha, tsk_env.data_dir());
        let task2 = create_test_task("task-2", test_repo.path(), &commit_sha, tsk_env.data_dir());

        // Set up storage with the first task
        let storage = get_task_storage(tsk_env);
        storage.add_task(task1.clone()).await.unwrap();

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let mut scheduler = TaskScheduler::new(
            Arc::new(ctx),
            test_docker_client(),
            storage.clone(),
            false,
            quit_signal,
        );

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
        storage.add_task(task2.clone()).await.unwrap();

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

        let storage = get_task_storage(ctx.tsk_env());

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let scheduler = TaskScheduler::new(ctx, test_docker_client(), storage, false, quit_signal);

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
        let storage = get_task_storage(ctx.tsk_env());
        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let scheduler = TaskScheduler::new(
            Arc::new(ctx),
            test_docker_client(),
            storage,
            false,
            quit_signal,
        );

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

    /// Create a test task with a parent.
    fn create_child_task(
        id: &str,
        repo_path: &std::path::Path,
        commit_sha: &str,
        parent_id: &str,
    ) -> Task {
        Task {
            id: id.to_string(),
            repo_root: repo_path.to_path_buf(),
            name: format!("task-{id}"),
            task_type: "test".to_string(),
            branch_name: format!("tsk/test/{id}"),
            source_commit: commit_sha.to_string(),
            source_branch: None,
            stack: "default".to_string(),
            project: "default".to_string(),
            copied_repo_path: None,
            parent_ids: vec![parent_id.to_string()],
            ..Task::test_default()
        }
    }

    #[test]
    fn test_is_parent_ready_no_parent() {
        // Task with no parent should return None
        let task = Task {
            id: "task-1".to_string(),
            branch_name: "tsk/test/task-1".to_string(),
            ..Task::test_default()
        };

        let all_tasks = vec![task.clone()];
        let result = TaskScheduler::is_parent_ready(&task, &all_tasks);
        assert!(result.is_none(), "Task with no parent should return None");
    }

    #[test]
    fn test_is_parent_ready_complete() {
        // Create a completed parent task
        let parent_task = Task {
            id: "parent-1".to_string(),
            name: "parent-task".to_string(),
            branch_name: "tsk/test/parent-1".to_string(),
            status: TaskStatus::Complete,
            ..Task::test_default()
        };

        // Create a child task
        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["parent-1".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![parent_task.clone(), child_task.clone()];
        let result = TaskScheduler::is_parent_ready(&child_task, &all_tasks);

        match result {
            Some(ParentStatus::Ready(parent)) => {
                assert_eq!(parent.id, "parent-1");
            }
            _ => panic!("Expected ParentStatus::Ready, got {:?}", result),
        }
    }

    #[test]
    fn test_is_parent_ready_waiting() {
        // Create a running parent task
        let parent_task = Task {
            id: "parent-1".to_string(),
            name: "parent-task".to_string(),
            branch_name: "tsk/test/parent-1".to_string(),
            status: TaskStatus::Running,
            ..Task::test_default()
        };

        // Create a child task
        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["parent-1".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![parent_task.clone(), child_task.clone()];
        let result = TaskScheduler::is_parent_ready(&child_task, &all_tasks);

        assert!(
            matches!(result, Some(ParentStatus::Waiting)),
            "Expected ParentStatus::Waiting, got {:?}",
            result
        );
    }

    #[test]
    fn test_is_parent_ready_failed() {
        // Create a failed parent task
        let parent_task = Task {
            id: "parent-1".to_string(),
            name: "parent-task".to_string(),
            branch_name: "tsk/test/parent-1".to_string(),
            status: TaskStatus::Failed,
            ..Task::test_default()
        };

        // Create a child task
        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["parent-1".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![parent_task.clone(), child_task.clone()];
        let result = TaskScheduler::is_parent_ready(&child_task, &all_tasks);

        match result {
            Some(ParentStatus::Failed(msg)) => {
                assert!(
                    msg.contains("parent-1"),
                    "Error message should mention parent task ID"
                );
            }
            _ => panic!("Expected ParentStatus::Failed, got {:?}", result),
        }
    }

    #[test]
    fn test_is_parent_ready_not_found() {
        // Create a child task with non-existent parent
        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["nonexistent-parent".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![child_task.clone()];
        let result = TaskScheduler::is_parent_ready(&child_task, &all_tasks);

        match result {
            Some(ParentStatus::NotFound(id)) => {
                assert_eq!(id, "nonexistent-parent");
            }
            _ => panic!("Expected ParentStatus::NotFound, got {:?}", result),
        }
    }

    #[test]
    fn test_is_task_ready_for_scheduling_no_parent() {
        // Task with no parent should be ready
        let task = Task {
            id: "task-1".to_string(),
            branch_name: "tsk/test/task-1".to_string(),
            ..Task::test_default()
        };

        let all_tasks = vec![task.clone()];
        let submitted = HashSet::new();
        let result = TaskScheduler::is_task_ready_for_scheduling(&task, &all_tasks, &submitted);
        assert!(result, "Task with no parent should be ready");
    }

    #[test]
    fn test_is_task_ready_for_scheduling_already_submitted() {
        // Task that's already submitted should not be ready
        let task = Task {
            id: "task-1".to_string(),
            branch_name: "tsk/test/task-1".to_string(),
            ..Task::test_default()
        };

        let all_tasks = vec![task.clone()];
        let mut submitted = HashSet::new();
        submitted.insert("task-1".to_string());
        let result = TaskScheduler::is_task_ready_for_scheduling(&task, &all_tasks, &submitted);
        assert!(!result, "Already submitted task should not be ready");
    }

    #[test]
    fn test_is_task_ready_for_scheduling_parent_waiting() {
        // Task with running parent should not be ready
        let parent_task = Task {
            id: "parent-1".to_string(),
            name: "parent-task".to_string(),
            branch_name: "tsk/test/parent-1".to_string(),
            status: TaskStatus::Running,
            ..Task::test_default()
        };

        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["parent-1".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![parent_task.clone(), child_task.clone()];
        let submitted = HashSet::new();
        let result =
            TaskScheduler::is_task_ready_for_scheduling(&child_task, &all_tasks, &submitted);
        assert!(!result, "Task with running parent should not be ready");
    }

    #[test]
    fn test_is_task_ready_for_scheduling_parent_complete() {
        // Task with completed parent should be ready
        let parent_task = Task {
            id: "parent-1".to_string(),
            name: "parent-task".to_string(),
            branch_name: "tsk/test/parent-1".to_string(),
            status: TaskStatus::Complete,
            ..Task::test_default()
        };

        let child_task = Task {
            id: "child-1".to_string(),
            name: "child-task".to_string(),
            branch_name: "tsk/test/child-1".to_string(),
            source_branch: None,
            copied_repo_path: None,
            parent_ids: vec!["parent-1".to_string()],
            ..Task::test_default()
        };

        let all_tasks = vec![parent_task.clone(), child_task.clone()];
        let submitted = HashSet::new();
        let result =
            TaskScheduler::is_task_ready_for_scheduling(&child_task, &all_tasks, &submitted);
        assert!(result, "Task with completed parent should be ready");
    }

    #[tokio::test]
    async fn test_fail_child_tasks() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let (test_repo, commit_sha) = setup_test_repo().unwrap();

        // Create parent and child tasks
        let parent_task = create_test_task(
            "parent-1",
            test_repo.path(),
            &commit_sha,
            tsk_env.data_dir(),
        );
        let child_task = create_child_task("child-1", test_repo.path(), &commit_sha, "parent-1");

        // Set up storage with both tasks
        let storage = get_task_storage(tsk_env);
        storage.add_task(parent_task.clone()).await.unwrap();
        storage.add_task(child_task.clone()).await.unwrap();

        let quit_signal = Arc::new(tokio::sync::Notify::new());
        let scheduler = TaskScheduler::new(
            Arc::new(ctx),
            test_docker_client(),
            storage.clone(),
            false,
            quit_signal,
        );

        // Get all tasks
        let tasks = storage.list_tasks().await.unwrap();

        // Fail child tasks
        scheduler
            .fail_child_tasks("parent-1", &tasks)
            .await
            .unwrap();

        // Verify child task is marked as failed
        let child = storage.get_task("child-1").await.unwrap().unwrap();
        assert_eq!(
            child.status,
            TaskStatus::Failed,
            "Child task should be marked as failed"
        );
        assert!(
            child.error_message.as_ref().unwrap().contains("parent-1"),
            "Error message should mention parent task"
        );
    }
}
