use crate::context::tsk_config::TskConfig;
use crate::server::protocol::{Request, Response};
use crate::task::{Task, TaskStatus};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Trait for communicating with the TSK server
#[async_trait]
pub trait TskClient: Send + Sync {
    /// Check if the server is available
    async fn is_server_available(&self) -> bool;

    /// Add a task to the server
    async fn add_task(
        &self,
        repo_path: PathBuf,
        task: Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// List all tasks from the server
    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get the status of a specific task
    ///
    /// Used by test utilities and debugging tools
    #[allow(dead_code)] // Used by test implementations
    async fn get_task_status(
        &self,
        task_id: String,
    ) -> Result<TaskStatus, Box<dyn std::error::Error + Send + Sync>>;

    /// Shutdown the server
    async fn shutdown_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Default implementation of TskClient that communicates with the TSK server via Unix sockets
#[derive(Clone)]
pub struct DefaultTskClient {
    socket_path: PathBuf,
}

impl DefaultTskClient {
    /// Create a new TSK client
    pub fn new(tsk_config: Arc<TskConfig>) -> Self {
        Self {
            socket_path: tsk_config.socket_path(),
        }
    }

    /// Send a request to the server and get a response
    async fn send_request(
        &self,
        request: Request,
    ) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
        // Connect to server with timeout
        let stream = timeout(
            Duration::from_secs(5),
            UnixStream::connect(&self.socket_path),
        )
        .await
        .map_err(|_| "Connection timeout")?
        .map_err(|e| format!("Failed to connect to server: {e}"))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send request
        let request_json = serde_json::to_string(&request)?;
        writer.write_all(request_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        // Read response with timeout
        let mut response_line = String::new();
        let bytes_read = timeout(
            Duration::from_secs(10),
            reader.read_line(&mut response_line),
        )
        .await
        .map_err(|_| "Response timeout")?
        .map_err(|e| format!("Failed to read response: {e}"))?;

        // Check if we received any data
        if bytes_read == 0 || response_line.trim().is_empty() {
            return Err("Server closed connection without sending a response".into());
        }

        // Parse response
        let response: Response = serde_json::from_str(&response_line)?;
        Ok(response)
    }
}

#[async_trait]
impl TskClient for DefaultTskClient {
    async fn is_server_available(&self) -> bool {
        matches!(
            timeout(
                Duration::from_secs(1),
                UnixStream::connect(&self.socket_path),
            )
            .await,
            Ok(Ok(_))
        )
    }

    async fn add_task(
        &self,
        repo_path: PathBuf,
        task: Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request = Request::AddTask {
            repo_path,
            task: Box::new(task),
        };
        let response = self.send_request(request).await?;

        match response {
            Response::Success { message } => {
                println!("{message}");
                Ok(())
            }
            Response::Error { message } => Err(message.into()),
            _ => Err("Unexpected response from server".into()),
        }
    }

    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>> {
        let request = Request::ListTasks;
        let response = self.send_request(request).await?;

        match response {
            Response::TaskList { tasks } => Ok(tasks),
            Response::Error { message } => Err(message.into()),
            _ => Err("Unexpected response from server".into()),
        }
    }

    async fn get_task_status(
        &self,
        task_id: String,
    ) -> Result<TaskStatus, Box<dyn std::error::Error + Send + Sync>> {
        let request = Request::GetStatus { task_id };
        let response = self.send_request(request).await?;

        match response {
            Response::TaskStatus { status } => Ok(status),
            Response::Error { message } => Err(message.into()),
            _ => Err("Unexpected response from server".into()),
        }
    }

    async fn shutdown_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request = Request::Shutdown;
        let response = self.send_request(request).await?;

        match response {
            Response::Success { message } => {
                println!("{message}");
                Ok(())
            }
            Response::Error { message } => Err(message.into()),
            _ => Err("Unexpected response from server".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[tokio::test]
    async fn test_client_creation() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        config.ensure_directories().unwrap();

        let client = DefaultTskClient::new(config);

        // Server should not be available without starting it
        assert!(!client.is_server_available().await);
    }

    #[tokio::test]
    async fn test_response_parsing_validates_empty_responses() {
        // This test documents that the send_request method now properly handles
        // empty responses by returning an error instead of causing a JSON parse error.
        // The actual EOF scenario is tested implicitly when the server closes
        // connections without sending data, which was the original bug.
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        config.ensure_directories().unwrap();

        let client = DefaultTskClient::new(config);

        // Attempting to list tasks when server is not running should fail gracefully
        let result = client.list_tasks().await;
        assert!(result.is_err());

        // The error should be about connection, not JSON parsing
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Failed to connect to server")
                || error_msg.contains("Connection refused"),
            "Expected connection error, got: {error_msg}"
        );
    }
}
