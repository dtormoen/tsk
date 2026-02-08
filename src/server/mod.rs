pub mod lifecycle;
pub mod scheduler;
pub mod worker_pool;

use crate::context::AppContext;
use crate::task_storage::get_task_storage;
use lifecycle::ServerLifecycle;
use scheduler::TaskScheduler;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Main TSK server that handles task management
pub struct TskServer {
    app_context: Arc<AppContext>,
    shutdown_signal: Arc<Mutex<bool>>,
    quit_signal: Arc<tokio::sync::Notify>,
    scheduler: Arc<Mutex<TaskScheduler>>,
    lifecycle: ServerLifecycle,
    workers: u32,
}

impl TskServer {
    /// Create a new TSK server instance with specified number of workers
    pub fn with_workers(app_context: Arc<AppContext>, workers: u32, quit_when_done: bool) -> Self {
        let tsk_env = app_context.tsk_env();
        let storage = get_task_storage(tsk_env.clone());

        // Create the quit signal for scheduler-to-server communication
        let quit_signal = Arc::new(tokio::sync::Notify::new());

        let scheduler = Arc::new(Mutex::new(TaskScheduler::new(
            app_context.clone(),
            storage.clone(),
            quit_when_done,
            quit_signal.clone(),
        )));
        let lifecycle = ServerLifecycle::new(tsk_env);

        Self {
            app_context,
            shutdown_signal: Arc::new(Mutex::new(false)),
            quit_signal,
            scheduler,
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

        loop {
            tokio::select! {
                _ = self.quit_signal.notified() => {
                    println!("Received quit signal from scheduler...");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if *self.shutdown_signal.lock().await {
                        println!("Server shutting down...");
                        break;
                    }
                }
            }
        }

        // Stop the scheduler
        self.scheduler.lock().await.stop().await;
        scheduler_handle.await?;

        // Clean up server resources
        self.lifecycle.cleanup()?;

        // Ensure terminal title is restored
        self.app_context.terminal_operations().restore_title();

        Ok(())
    }

    /// Signal the server to shut down
    pub async fn shutdown(&self) {
        *self.shutdown_signal.lock().await = true;
    }
}
