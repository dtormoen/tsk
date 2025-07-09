use crate::context::tsk_client::TskClient;
use crate::task::{Task, TaskStatus};
use async_trait::async_trait;
use std::path::PathBuf;

/// A no-op implementation of TskClient for testing
#[derive(Clone)]
pub struct NoOpTskClient;

#[async_trait]
impl TskClient for NoOpTskClient {
    async fn is_server_available(&self) -> bool {
        false
    }

    async fn add_task(
        &self,
        _repo_path: PathBuf,
        _task: Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![])
    }

    async fn get_task_status(
        &self,
        _task_id: String,
    ) -> Result<TaskStatus, Box<dyn std::error::Error + Send + Sync>> {
        Ok(TaskStatus::Queued)
    }

    async fn shutdown_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}
