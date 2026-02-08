use crate::context::tsk_client::TskClient;
use async_trait::async_trait;

/// A no-op implementation of TskClient for testing
#[derive(Clone)]
pub struct NoOpTskClient;

#[async_trait]
impl TskClient for NoOpTskClient {
    async fn is_server_available(&self) -> bool {
        false
    }

    async fn shutdown_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}
