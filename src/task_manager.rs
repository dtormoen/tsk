use crate::context::{file_system::FileSystemOperations, AppContext};
use crate::docker::DockerManager;
use crate::git::get_repo_manager;
use crate::task::{Task, TaskStatus};
use crate::task_runner::{TaskExecutionError, TaskExecutionResult, TaskRunner};
use crate::task_storage::{get_task_storage, TaskStorage};
use std::path::PathBuf;
use std::sync::Arc;

pub struct TaskManager {
    task_runner: TaskRunner,
    task_storage: Option<Box<dyn TaskStorage>>,
    file_system: Arc<dyn FileSystemOperations>,
    repo_root: Option<PathBuf>,
}

impl TaskManager {
    pub fn new(ctx: &AppContext) -> Result<Self, String> {
        let repo_manager = get_repo_manager(ctx.file_system(), ctx.git_operations());
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(repo_manager, docker_manager, ctx.file_system());

        Ok(Self {
            task_runner,
            task_storage: None,
            file_system: ctx.file_system(),
            repo_root: None,
        })
    }

    pub fn with_storage(ctx: &AppContext) -> Result<Self, String> {
        let repo_manager = get_repo_manager(ctx.file_system(), ctx.git_operations());
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(repo_manager, docker_manager, ctx.file_system());

        Ok(Self {
            task_runner,
            task_storage: Some(get_task_storage(ctx.file_system())),
            file_system: ctx.file_system(),
            repo_root: None,
        })
    }

    /// Get the repository root path
    fn get_repo_root(&self) -> Result<PathBuf, String> {
        match &self.repo_root {
            Some(root) => Ok(root.clone()),
            None => std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e)),
        }
    }

    /// Execute a task from the queue (with status updates)
    pub async fn execute_queued_task(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Update task status to running if we have storage
        if let Some(ref storage) = self.task_storage {
            let mut running_task = task.clone();
            running_task.status = TaskStatus::Running;
            running_task.started_at = Some(chrono::Utc::now());

            if let Err(e) = storage.update_task(running_task.clone()).await {
                eprintln!("Error updating task status: {}", e);
            }
        }

        // Execute the task
        let execution_result = self.task_runner.execute_task(task).await;

        match execution_result {
            Ok(result) => {
                // Update task status based on the task result if we have storage
                if let Some(ref storage) = self.task_storage {
                    let mut updated_task = task.clone();
                    updated_task.completed_at = Some(chrono::Utc::now());
                    updated_task.branch_name = Some(result.branch_name.clone());

                    // Check if we have a parsed result from the log processor
                    if let Some(task_result) = result.task_result.as_ref() {
                        if task_result.success {
                            updated_task.status = TaskStatus::Complete;
                        } else {
                            updated_task.status = TaskStatus::Failed;
                            updated_task.error_message = Some(task_result.message.clone());
                        }
                    } else {
                        // Default to complete if no explicit result was found
                        updated_task.status = TaskStatus::Complete;
                    }

                    if let Err(e) = storage.update_task(updated_task).await {
                        eprintln!("Error updating task status: {}", e);
                    }
                }
                Ok(result)
            }
            Err(e) => {
                // Update task status to failed if we have storage
                if let Some(ref storage) = self.task_storage {
                    let mut failed_task = task.clone();
                    failed_task.status = TaskStatus::Failed;
                    failed_task.error_message = Some(e.message.clone());
                    failed_task.completed_at = Some(chrono::Utc::now());

                    if let Err(storage_err) = storage.update_task(failed_task).await {
                        eprintln!("Error updating task status: {}", storage_err);
                    }
                }
                Err(e)
            }
        }
    }

    /// Delete a specific task and its associated directory
    pub async fn delete_task(&self, task_id: &str) -> Result<(), String> {
        // Get task storage to delete from database
        let storage = match &self.task_storage {
            Some(s) => s,
            None => return Err("Task storage not initialized".to_string()),
        };

        // Get the task to find its directory
        let task = storage
            .get_task(task_id)
            .await
            .map_err(|e| format!("Error getting task: {}", e))?;

        if task.is_none() {
            return Err(format!("Task with ID '{}' not found", task_id));
        }

        // Delete from storage first
        storage
            .delete_task(task_id)
            .await
            .map_err(|e| format!("Error deleting task from storage: {}", e))?;

        // Delete the task directory
        let repo_root = self.get_repo_root()?;
        let task_dir = repo_root.join(".tsk/tasks").join(task_id);
        if self.file_system.exists(&task_dir).await.unwrap_or(false) {
            self.file_system
                .remove_dir(&task_dir)
                .await
                .map_err(|e| format!("Error deleting task directory: {}", e))?;
        }

        Ok(())
    }

    /// Delete all completed tasks and all quick tasks
    pub async fn clean_tasks(&self) -> Result<(usize, usize), String> {
        // Get task storage
        let storage = match &self.task_storage {
            Some(s) => s,
            None => return Err("Task storage not initialized".to_string()),
        };

        // Get all tasks to find directories to delete
        let all_tasks = storage
            .list_tasks()
            .await
            .map_err(|e| format!("Error listing tasks: {}", e))?;

        // Filter completed tasks
        let completed_tasks: Vec<&Task> = all_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Complete)
            .collect();

        // Delete completed tasks directories
        let repo_root = self.get_repo_root()?;
        for task in &completed_tasks {
            let task_dir = repo_root.join(".tsk/tasks").join(&task.id);
            if self.file_system.exists(&task_dir).await.unwrap_or(false) {
                if let Err(e) = self.file_system.remove_dir(&task_dir).await {
                    eprintln!(
                        "Warning: Failed to delete task directory {}: {}",
                        task.id, e
                    );
                }
            }
        }

        // Delete completed tasks from storage
        let deleted_count = storage
            .delete_tasks_by_status(vec![TaskStatus::Complete])
            .await
            .map_err(|e| format!("Error deleting completed tasks: {}", e))?;

        // Delete all quick task directories
        let quick_tasks_dir = repo_root.join(".tsk/quick-tasks");
        let mut quick_task_count = 0;
        if self
            .file_system
            .exists(&quick_tasks_dir)
            .await
            .unwrap_or(false)
        {
            match self.file_system.read_dir(&quick_tasks_dir).await {
                Ok(entries) => {
                    for entry_path in entries {
                        if let Err(e) = self.file_system.remove_dir(&entry_path).await {
                            eprintln!("Warning: Failed to delete quick task directory: {}", e);
                        } else {
                            quick_task_count += 1;
                        }
                    }
                }
                Err(e) => eprintln!("Warning: Failed to read quick tasks directory: {}", e),
            }
        }

        Ok((deleted_count, quick_task_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::docker_client::DockerClient;
    use async_trait::async_trait;
    use std::sync::Arc;

    // Mock Docker client for tests
    #[derive(Clone)]
    struct MockDockerClient {
        container_id: String,
        exit_code: i64,
        logs_output: String,
    }

    impl MockDockerClient {
        fn new() -> Self {
            Self {
                container_id: "test-container-id".to_string(),
                exit_code: 0,
                logs_output: "Test output".to_string(),
            }
        }
    }

    #[async_trait]
    impl DockerClient for MockDockerClient {
        #[cfg(test)]
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        async fn create_container(
            &self,
            _options: Option<bollard::container::CreateContainerOptions<String>>,
            _config: bollard::container::Config<String>,
        ) -> Result<String, String> {
            Ok(self.container_id.clone())
        }

        async fn start_container(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn wait_container(&self, _id: &str) -> Result<i64, String> {
            Ok(self.exit_code)
        }

        async fn logs(
            &self,
            _id: &str,
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<String, String> {
            Ok(self.logs_output.clone())
        }

        async fn logs_stream(
            &self,
            _id: &str,
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<
            Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>,
            String,
        > {
            // Return a simple stream with the test output
            use futures_util::stream::StreamExt;
            let output = self.logs_output.clone();
            let stream = futures_util::stream::once(async move { Ok(output) }).boxed();
            Ok(Box::new(stream))
        }

        async fn remove_container(
            &self,
            _id: &str,
            _options: Option<bollard::container::RemoveContainerOptions>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn create_network(&self, _name: &str) -> Result<String, String> {
            Ok("test-network".to_string())
        }

        async fn network_exists(&self, _name: &str) -> Result<bool, String> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn test_delete_task() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::context::git_operations::tests::MockGitOperations;

        // Create a task to test with
        let task_id = "test-task-123".to_string();
        let _task = Task::new_with_id(
            task_id.clone(),
            "test-task".to_string(),
            "feature".to_string(),
            Some("Test description".to_string()),
            None,
            None,
            30,
        );
        let current_dir = std::env::current_dir().unwrap();
        let task_dir_path = current_dir.join(".tsk/tasks").join(&task_id);

        // Create mock file system with necessary structure
        let git_dir = current_dir.join(".git");
        let tsk_dir = current_dir.join(".tsk");
        let tasks_dir = current_dir.join(".tsk/tasks");
        let tasks_json_path = ".tsk/tasks.json"; // Keep this relative for JsonTaskStorage

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&tsk_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&task_dir_path.to_string_lossy().to_string())
                .with_file(&format!("{}/test.txt", task_dir_path.to_string_lossy()), "test content")
                .with_file(&tasks_json_path, &format!(r#"[{{"id":"{}","name":"test-task","task_type":"feature","description":"Test description","instructions_file":null,"agent":null,"timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":null,"error_message":null}}]"#, task_id))
        );

        let docker_client = Arc::new(MockDockerClient::new());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Delete the task
        let result = task_manager.delete_task(&task_id).await;
        assert!(result.is_ok(), "Failed to delete task: {:?}", result);

        // Verify task directory is deleted by checking the mock file system
        let exists = fs.exists(&task_dir_path).await.unwrap();
        assert!(!exists, "Task directory should have been deleted");
    }

    #[tokio::test]
    async fn test_clean_tasks() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::context::git_operations::tests::MockGitOperations;

        // Create tasks with different statuses
        let queued_task_id = "queued-task-123".to_string();
        let completed_task_id = "completed-task-456".to_string();

        let _queued_task = Task::new_with_id(
            queued_task_id.clone(),
            "queued-task".to_string(),
            "feature".to_string(),
            Some("Queued task".to_string()),
            None,
            None,
            30,
        );

        let mut completed_task = Task::new_with_id(
            completed_task_id.clone(),
            "completed-task".to_string(),
            "bug-fix".to_string(),
            Some("Completed task".to_string()),
            None,
            None,
            30,
        );
        completed_task.status = TaskStatus::Complete;

        let current_dir = std::env::current_dir().unwrap();
        let queued_dir_path = current_dir.join(".tsk/tasks").join(&queued_task_id);
        let completed_dir_path = current_dir.join(".tsk/tasks").join(&completed_task_id);

        // Create initial tasks.json with both tasks
        let tasks_json = format!(
            r#"[{{"id":"{}","name":"queued-task","task_type":"feature","description":"Queued task","instructions_file":null,"agent":null,"timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":null,"error_message":null}},{{"id":"{}","name":"completed-task","task_type":"bug-fix","description":"Completed task","instructions_file":null,"agent":null,"timeout":30,"status":"COMPLETE","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":"2024-01-01T01:00:00Z","branch_name":null,"error_message":null}}]"#,
            queued_task_id, completed_task_id
        );

        // Create mock file system with necessary structure
        let git_dir = current_dir.join(".git");
        let tsk_dir = current_dir.join(".tsk");
        let tasks_dir = current_dir.join(".tsk/tasks");
        let quick_tasks_dir = current_dir.join(".tsk/quick-tasks");
        let quick_task_dir1 = current_dir.join(".tsk/quick-tasks/2024-01-01-1200-quick1");
        let quick_task_dir2 = current_dir.join(".tsk/quick-tasks/2024-01-01-1300-quick2");
        let tasks_json_path = ".tsk/tasks.json"; // Keep relative for JsonTaskStorage

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&tsk_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&quick_tasks_dir.to_string_lossy().to_string())
                .with_dir(&queued_dir_path.to_string_lossy().to_string())
                .with_dir(&completed_dir_path.to_string_lossy().to_string())
                .with_dir(&quick_task_dir1.to_string_lossy().to_string())
                .with_dir(&quick_task_dir2.to_string_lossy().to_string())
                .with_file(&tasks_json_path, &tasks_json),
        );

        let docker_client = Arc::new(MockDockerClient::new());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let (completed_count, quick_count) = result.unwrap();
        assert_eq!(completed_count, 1);
        assert_eq!(quick_count, 2);

        // Verify directories are cleaned up
        let queued_exists = fs.exists(&queued_dir_path).await.unwrap();
        let completed_exists = fs.exists(&completed_dir_path).await.unwrap();
        assert!(queued_exists, "Queued task directory should still exist");
        assert!(
            !completed_exists,
            "Completed task directory should be deleted"
        );
    }

    #[tokio::test]
    async fn test_clean_tasks_with_id_matching() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::context::git_operations::tests::MockGitOperations;

        // Create a task with a specific ID using new_with_id
        let task_id = "2024-01-15-1430-test-feature".to_string();
        let mut completed_task = Task::new_with_id(
            task_id.clone(),
            "test-feature".to_string(),
            "feature".to_string(),
            Some("Test feature".to_string()),
            None,
            None,
            30,
        );
        completed_task.status = TaskStatus::Complete;

        let current_dir = std::env::current_dir().unwrap();
        let task_dir_path = current_dir.join(".tsk/tasks").join(&task_id);

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","name":"test-feature","task_type":"feature","description":"Test feature","instructions_file":null,"agent":null,"timeout":30,"status":"COMPLETE","created_at":"2024-01-15T14:30:00Z","started_at":null,"completed_at":"2024-01-15T15:00:00Z","branch_name":null,"error_message":null}}]"#,
            task_id
        );

        // Create mock file system with necessary structure
        let git_dir = current_dir.join(".git");
        let tsk_dir = current_dir.join(".tsk");
        let tasks_dir = current_dir.join(".tsk/tasks");
        let tasks_json_path = ".tsk/tasks.json"; // Keep relative for JsonTaskStorage

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&tsk_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&task_dir_path.to_string_lossy().to_string())
                .with_file(
                    &format!("{}/instructions.md", task_dir_path.to_string_lossy()),
                    "Test instructions",
                )
                .with_file(&tasks_json_path, &tasks_json),
        );

        let docker_client = Arc::new(MockDockerClient::new());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let (completed_count, _) = result.unwrap();
        assert_eq!(completed_count, 1);

        // Verify directory was deleted
        let task_dir_exists = fs.exists(&task_dir_path).await.unwrap();
        assert!(!task_dir_exists, "Task directory should have been deleted");
    }
}
