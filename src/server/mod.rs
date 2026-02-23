pub mod lifecycle;
pub mod scheduler;
pub mod worker_pool;

use crate::context::AppContext;
use crate::context::docker_client::DockerClient;
use crate::docker::proxy_manager::ProxyManager;
use crate::task::TaskStatus;
use crate::tui::events::ServerEventSender;
use lifecycle::ServerLifecycle;
use scheduler::TaskScheduler;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Main TSK server that handles task management
pub struct TskServer {
    app_context: Arc<AppContext>,
    docker_client: Arc<dyn DockerClient>,
    quit_signal: Arc<tokio::sync::Notify>,
    scheduler: Arc<Mutex<TaskScheduler>>,
    scheduler_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Handle to scheduler's submitted task IDs, extracted before the scheduler mutex is held
    submitted_tasks: Arc<Mutex<HashSet<String>>>,
    /// Handle to scheduler's running flag, extracted before the scheduler mutex is held
    running_flag: Arc<Mutex<bool>>,
    lifecycle: ServerLifecycle,
    workers: u32,
}

impl TskServer {
    /// Create a new TSK server instance with specified number of workers
    pub fn with_workers(
        app_context: Arc<AppContext>,
        docker_client: Arc<dyn DockerClient>,
        workers: u32,
        quit_when_done: bool,
        event_sender: Option<ServerEventSender>,
    ) -> Self {
        let tsk_env = app_context.tsk_env();
        let storage = app_context.task_storage();

        // Create the quit signal for scheduler-to-server communication
        let quit_signal = Arc::new(tokio::sync::Notify::new());

        let scheduler = TaskScheduler::new(
            app_context.clone(),
            docker_client.clone(),
            storage.clone(),
            quit_when_done,
            quit_signal.clone(),
            event_sender.clone(),
        );

        // Extract shared state handles before wrapping scheduler in Mutex.
        // This allows graceful_shutdown() to operate without acquiring the scheduler lock.
        let submitted_tasks = scheduler.submitted_task_ids();
        let running_flag = scheduler.running_flag();

        let scheduler = Arc::new(Mutex::new(scheduler));
        let lifecycle = ServerLifecycle::new(tsk_env);

        Self {
            app_context,
            docker_client,
            quit_signal,
            scheduler,
            scheduler_handle: Mutex::new(None),
            submitted_tasks,
            running_flag,
            lifecycle,
            workers,
        }
    }

    /// Start the server and process tasks
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if server is already running
        if self.lifecycle.is_server_running() {
            return Err("Server is already running".into());
        }

        // Write PID file
        self.lifecycle.write_pid()?;

        println!("TSK Server started (PID {})", std::process::id());

        // Start the task scheduler in the background
        let scheduler = self.scheduler.clone();
        let workers = self.workers;
        let scheduler_handle = tokio::spawn(async move {
            if let Err(e) = scheduler.lock().await.start(workers).await {
                eprintln!("Scheduler error: {e}");
            }
        });
        *self.scheduler_handle.lock().await = Some(scheduler_handle);

        // Wait for the quit signal from the scheduler (quit-when-done mode)
        self.quit_signal.notified().await;
        println!("Received quit signal from scheduler...");

        // Wait for the scheduler task to finish its cleanup
        if let Some(handle) = self.scheduler_handle.lock().await.take() {
            let _ = handle.await;
        }

        // Clean up server resources
        self.lifecycle.cleanup()?;

        // Ensure terminal title is restored
        self.app_context.terminal_operations().restore_title();

        Ok(())
    }

    /// Perform graceful shutdown: kill managed containers, drain pool, mark tasks failed, cleanup.
    ///
    /// This is called when the server receives a shutdown signal (SIGINT/SIGTERM).
    /// It stops the scheduler, kills any running containers managed by the server,
    /// waits for the worker pool to drain (workers handle their own network cleanup
    /// as part of their normal exit path), marks remaining running tasks as failed,
    /// conditionally stops the proxy, and cleans up server resources.
    ///
    /// Uses pre-extracted Arc handles to avoid acquiring the scheduler mutex, which
    /// is held for the entire lifetime of the scheduler's `start()` loop.
    pub async fn graceful_shutdown(&self) {
        *self.running_flag.lock().await = false;

        let task_ids: Vec<String> = self.submitted_tasks.lock().await.iter().cloned().collect();

        let proxy_manager = ProxyManager::new(
            self.docker_client.clone(),
            self.app_context.tsk_env(),
            self.app_context.tsk_config().container_engine.clone(),
        );

        if !task_ids.is_empty() {
            let docker_client = &self.docker_client;
            for id in &task_ids {
                let container_name = format!("tsk-{id}");
                if let Err(e) = docker_client.kill_container(&container_name).await {
                    eprintln!("Note: Could not kill container {container_name}: {e}");
                }
            }
        }

        // Wait for the scheduler task to complete (it drains the worker pool internally)
        if let Some(handle) = self.scheduler_handle.lock().await.take()
            && tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                .await
                .is_err()
        {
            eprintln!("Warning: Scheduler did not stop within 5 seconds");
        }

        let storage = self.app_context.task_storage();
        let mut stopped_fingerprints = std::collections::HashSet::new();
        for task_id in &task_ids {
            if let Ok(Some(task)) = storage.get_task(task_id).await {
                if task.status == TaskStatus::Running {
                    let _ = storage.mark_failed(task_id, "Server shutdown").await;
                }
                // Collect unique proxy configs and stop idle proxies
                let resolved = crate::docker::resolve_config_from_task(&task, &self.app_context);
                let proxy_config = resolved.proxy_config();
                let fp = proxy_config.fingerprint();
                if stopped_fingerprints.insert(fp)
                    && let Err(e) = proxy_manager.maybe_stop_proxy(&proxy_config).await
                {
                    eprintln!("Warning: Failed to stop proxy during shutdown: {e}");
                }
            }
        }

        if let Err(e) = self.lifecycle.cleanup() {
            eprintln!("Warning: Failed to clean up PID file: {e}");
        }

        self.app_context.terminal_operations().restore_title();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use crate::test_utils::TrackedDockerClient;

    #[tokio::test]
    async fn test_graceful_shutdown_kills_containers_and_marks_tasks_failed() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = Arc::new(AppContext::builder().build());
        let server = TskServer::with_workers(ctx.clone(), mock_client.clone(), 1, false, None);

        // Add tasks to storage as Running
        let storage = ctx.task_storage();
        let task1 = Task {
            id: "task-1".to_string(),
            name: "test-task-1".to_string(),
            branch_name: "tsk/test/task-1".to_string(),
            ..Task::test_default()
        };
        let task2 = Task {
            id: "task-2".to_string(),
            name: "test-task-2".to_string(),
            branch_name: "tsk/test/task-2".to_string(),
            ..Task::test_default()
        };

        storage.add_task(task1).await.unwrap();
        storage.add_task(task2).await.unwrap();

        // Mark both tasks as Running
        storage.mark_running("task-1").await.unwrap();
        storage.mark_running("task-2").await.unwrap();

        // Simulate that these tasks are submitted to the scheduler
        {
            let mut submitted = server.submitted_tasks.lock().await;
            submitted.insert("task-1".to_string());
            submitted.insert("task-2".to_string());
        }

        // Perform graceful shutdown
        server.graceful_shutdown().await;

        // Verify kill_container was called for both tasks
        {
            let kill_calls = mock_client.kill_container_calls.lock().unwrap();
            assert_eq!(kill_calls.len(), 2);
            assert!(kill_calls.contains(&"tsk-task-1".to_string()));
            assert!(kill_calls.contains(&"tsk-task-2".to_string()));
        }

        // Network cleanup is handled by the worker pool's normal exit path
        // (run_task_container cleans up its own network after container exit),
        // so graceful_shutdown should NOT do network cleanup directly.
        {
            let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
            assert_eq!(disconnect_calls.len(), 0);
        }
        {
            let remove_calls = mock_client.remove_network_calls.lock().unwrap();
            assert_eq!(remove_calls.len(), 0);
        }

        // Verify both tasks are now marked as Failed
        let t1 = storage.get_task("task-1").await.unwrap().unwrap();
        assert_eq!(t1.status, TaskStatus::Failed);
        assert_eq!(t1.error_message.as_deref(), Some("Server shutdown"));

        let t2 = storage.get_task("task-2").await.unwrap().unwrap();
        assert_eq!(t2.status, TaskStatus::Failed);
        assert_eq!(t2.error_message.as_deref(), Some("Server shutdown"));
    }

    #[tokio::test]
    async fn test_graceful_shutdown_skips_completed_tasks() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = Arc::new(AppContext::builder().build());
        let server = TskServer::with_workers(ctx.clone(), mock_client.clone(), 1, false, None);

        // Add a task that's already completed
        let storage = ctx.task_storage();
        let task = Task {
            id: "task-done".to_string(),
            name: "done-task".to_string(),
            branch_name: "tsk/test/task-done".to_string(),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();
        storage
            .mark_complete("task-done", "tsk/test/task-done")
            .await
            .unwrap();

        // Simulate it was submitted (the worker completed it before shutdown)
        server
            .submitted_tasks
            .lock()
            .await
            .insert("task-done".to_string());

        server.graceful_shutdown().await;

        // Container kill is still attempted (container may still be running)
        {
            let kill_calls = mock_client.kill_container_calls.lock().unwrap();
            assert_eq!(kill_calls.len(), 1);
        }

        // Network cleanup is handled by the worker pool's normal exit path,
        // not by graceful_shutdown directly.
        {
            let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
            assert_eq!(disconnect_calls.len(), 0);
        }
        {
            let remove_calls = mock_client.remove_network_calls.lock().unwrap();
            assert_eq!(remove_calls.len(), 0);
        }

        // But task status should remain Complete (not overwritten to Failed)
        let t = storage.get_task("task-done").await.unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Complete);
    }
}
