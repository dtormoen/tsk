pub mod executor;
pub mod lifecycle;
pub mod protocol;

use crate::context::AppContext;
use crate::task_storage::{get_task_storage, TaskStorage};
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
    /// Create a new TSK server instance
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let xdg_directories = app_context.xdg_directories();
        let socket_path = xdg_directories.socket_path();
        let storage = get_task_storage(xdg_directories.clone(), app_context.file_system());
        let storage = Arc::new(Mutex::new(storage));

        let executor = Arc::new(TaskExecutor::new(app_context.clone(), storage.clone()));
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
                eprintln!("Executor error: {}", e);
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
                            eprintln!("Error handling client: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Error accepting connection: {}", e);
                }
            }
        }

        // Stop the executor
        self.executor.stop().await;
        executor_handle.await?;

        // Clean up server resources
        self.lifecycle.cleanup()?;

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
    reader.read_line(&mut line).await?;
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
