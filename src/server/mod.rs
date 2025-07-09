pub mod executor;
pub mod lifecycle;
pub mod protocol;

use crate::context::AppContext;
use crate::task_storage::{TaskStorage, get_task_storage};
use executor::TaskExecutor;
use lifecycle::ServerLifecycle;
use protocol::{Request, Response};
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
    executor: Arc<TaskExecutor>,
    lifecycle: ServerLifecycle,
}

impl TskServer {
    /// Create a new TSK server instance with specified number of workers
    pub fn with_workers(app_context: Arc<AppContext>, workers: u32) -> Self {
        let xdg_directories = app_context.xdg_directories();
        let socket_path = xdg_directories.socket_path();
        let storage = get_task_storage(xdg_directories.clone(), app_context.file_system());
        let storage = Arc::new(Mutex::new(storage));

        let executor = Arc::new(TaskExecutor::with_workers(
            app_context.clone(),
            storage.clone(),
            workers,
        ));
        let lifecycle = ServerLifecycle::new(xdg_directories);

        Self {
            app_context,
            storage,
            socket_path,
            shutdown_signal: Arc::new(Mutex::new(false)),
            executor,
            lifecycle,
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

        // Start the task executor in the background
        let executor = self.executor.clone();
        let executor_handle = tokio::spawn(async move {
            if let Err(e) = executor.start().await {
                eprintln!("Executor error: {e}");
            }
        });

        // Accept connections in a loop
        loop {
            // Check shutdown signal
            if *self.shutdown_signal.lock().await {
                println!("Server shutting down...");
                break;
            }

            match listener.accept().await {
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

        // Stop the executor
        self.executor.stop().await;
        executor_handle.await?;

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
    use crate::test_utils::NoOpDockerClient;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    /// Helper to create a test AppContext
    fn create_test_context() -> (Arc<AppContext>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = Arc::new(crate::storage::XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        let context = AppContext::builder()
            .with_docker_client(Arc::new(NoOpDockerClient))
            .with_xdg_directories(xdg)
            .build();

        (Arc::new(context), temp_dir)
    }

    #[tokio::test]
    async fn test_handle_client_with_empty_connection() {
        // This test verifies that the server gracefully handles connections
        // that don't send any data (like is_server_available checks)
        let (app_context, _temp_dir) = create_test_context();
        let storage = get_task_storage(app_context.xdg_directories(), app_context.file_system());
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
        let (app_context, _temp_dir) = create_test_context();
        let storage = get_task_storage(app_context.xdg_directories(), app_context.file_system());
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
