pub mod lifecycle;
pub mod protocol;
pub mod scheduler;
pub mod worker_pool;

use crate::context::AppContext;
use crate::task_storage::{TaskStorage, get_task_storage};
use lifecycle::ServerLifecycle;
use protocol::{Request, Response};
use scheduler::TaskScheduler;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

/// Main TSK server that handles task management
pub struct TskServer {
    app_context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    socket_path: PathBuf,
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
        let socket_path = tsk_env.socket_path();
        let storage = get_task_storage(tsk_env.clone(), app_context.file_system());
        let storage = Arc::new(Mutex::new(storage));

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
            storage,
            socket_path,
            shutdown_signal: Arc::new(Mutex::new(false)),
            quit_signal,
            scheduler,
            lifecycle,
            workers,
        }
    }

    /// Start the server and listen for connections
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if server is already running
        if self.lifecycle.is_server_running() {
            return Err("Server is already running".into());
        }

        // Write PID file
        self.lifecycle.write_pid()?;

        // Ensure socket doesn't already exist
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        // Create and bind to socket
        let listener = UnixListener::bind(&self.socket_path)?;
        println!("TSK Server listening on: {:?}", self.socket_path);

        // Start the task scheduler in the background
        let scheduler = self.scheduler.clone();
        let workers = self.workers;
        let scheduler_handle = tokio::spawn(async move {
            if let Err(e) = scheduler.lock().await.start(workers).await {
                eprintln!("Scheduler error: {e}");
            }
        });

        // Accept connections in a loop
        loop {
            tokio::select! {
                _ = self.quit_signal.notified() => {
                    println!("Received quit signal from scheduler...");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    // Periodically check shutdown signal
                    if *self.shutdown_signal.lock().await {
                        println!("Server shutting down...");
                        break;
                    }
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let app_context = self.app_context.clone();
                            let storage = self.storage.clone();
                            let shutdown_signal = self.shutdown_signal.clone();

                            // Handle each client in a separate task
                            tokio::spawn(async move {
                                if let Err(e) =
                                    handle_client(stream, app_context, storage, shutdown_signal).await
                                {
                                    eprintln!("Error handling client: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("Error accepting connection: {e}");
                        }
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

/// Handle a single client connection
async fn handle_client(
    stream: UnixStream,
    _app_context: Arc<AppContext>,
    storage: Arc<Mutex<Box<dyn TaskStorage>>>,
    shutdown_signal: Arc<Mutex<bool>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read request
    let bytes_read = reader.read_line(&mut line).await?;

    // Handle empty connections (e.g., from is_server_available checks)
    if bytes_read == 0 || line.trim().is_empty() {
        // Client connected but didn't send data - this is normal for availability checks
        return Ok(());
    }

    let request: Request = serde_json::from_str(&line)?;

    // Process request
    let response = match request {
        Request::AddTask { repo_path: _, task } => {
            let storage = storage.lock().await;
            match storage.add_task(*task).await {
                Ok(_) => Response::Success {
                    message: "Task added successfully".to_string(),
                },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::ListTasks => {
            let storage = storage.lock().await;
            match storage.list_tasks().await {
                Ok(tasks) => Response::TaskList { tasks },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::GetStatus { task_id } => {
            let storage = storage.lock().await;
            match storage.get_task(&task_id).await {
                Ok(Some(task)) => Response::TaskStatus {
                    status: task.status,
                },
                Ok(None) => Response::Error {
                    message: "Task not found".to_string(),
                },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::Shutdown => {
            *shutdown_signal.lock().await = true;
            Response::Success {
                message: "Server shutting down".to_string(),
            }
        }
    };

    // Send response
    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    /// Helper to create a test AppContext
    fn create_test_context() -> Arc<AppContext> {
        // Create AppContext - automatically gets test defaults
        Arc::new(AppContext::builder().build())
    }

    #[tokio::test]
    async fn test_handle_client_with_empty_connection() {
        // This test verifies that the server gracefully handles connections
        // that don't send any data (like is_server_available checks)
        let app_context = create_test_context();
        let storage = get_task_storage(app_context.tsk_env(), app_context.file_system());
        let storage = Arc::new(Mutex::new(storage));
        let shutdown_signal = Arc::new(Mutex::new(false));

        // Create a pair of connected Unix sockets
        let (client, server) = UnixStream::pair().unwrap();

        // Close the client side immediately (simulating is_server_available)
        drop(client);

        // Handle the server side - this should not error with EOF
        let result = handle_client(server, app_context, storage, shutdown_signal).await;

        // The function should return Ok(()) without panicking or erroring
        assert!(result.is_ok(), "Expected Ok(()), got {result:?}");
    }

    #[tokio::test]
    async fn test_handle_client_with_valid_request() {
        // This test verifies that valid requests still work correctly
        let app_context = create_test_context();
        let storage = get_task_storage(app_context.tsk_env(), app_context.file_system());
        let storage = Arc::new(Mutex::new(storage));
        let shutdown_signal = Arc::new(Mutex::new(false));

        // Create a pair of connected Unix sockets
        let (mut client, server) = UnixStream::pair().unwrap();

        // Send a valid request from the client side
        let request = Request::ListTasks;
        let request_json = serde_json::to_string(&request).unwrap();
        client.write_all(request_json.as_bytes()).await.unwrap();
        client.write_all(b"\n").await.unwrap();
        client.flush().await.unwrap();

        // Handle the request on the server side
        let server_handle = tokio::spawn(async move {
            handle_client(server, app_context, storage, shutdown_signal).await
        });

        // Read the response
        let mut buf = vec![0; 1024];
        let n = client.read(&mut buf).await.unwrap();
        let response_str = String::from_utf8_lossy(&buf[..n]);

        // Verify we got a valid response
        assert!(response_str.contains("TaskList") || response_str.contains("Error"));

        // Ensure the server handler completes successfully
        let result = server_handle.await.unwrap();
        assert!(result.is_ok());
    }
}
