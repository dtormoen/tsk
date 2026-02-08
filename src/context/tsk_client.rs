use crate::context::tsk_env::TskEnv;
use crate::server::protocol::{Request, Response};
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
    pub fn new(tsk_env: Arc<TskEnv>) -> Self {
        Self {
            socket_path: tsk_env.socket_path(),
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

    async fn shutdown_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request = Request::Shutdown;
        let response = self.send_request(request).await?;

        match response {
            Response::Success { message } => {
                println!("{message}");
                Ok(())
            }
            Response::Error { message } => Err(message.into()),
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
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let client = DefaultTskClient::new(tsk_env);

        // Server should not be available without starting it
        assert!(!client.is_server_available().await);
    }

    #[tokio::test]
    async fn test_shutdown_when_server_not_running() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let client = DefaultTskClient::new(tsk_env);

        // Attempting to shutdown when server is not running should fail gracefully
        let result = client.shutdown_server().await;
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
