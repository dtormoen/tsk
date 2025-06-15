use crate::context::{file_system::FileSystemOperations, AppContext};
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::repo_utils::find_repository_root;
use crate::storage::XdgDirectories;
use crate::task::{Task, TaskStatus};
use crate::task_runner::{TaskExecutionError, TaskExecutionResult, TaskRunner};
use crate::task_storage::{get_task_storage, TaskStorage};
use std::path::Path;
use std::sync::Arc;

pub struct TaskManager {
    task_runner: TaskRunner,
    task_storage: Option<Box<dyn TaskStorage>>,
    file_system: Arc<dyn FileSystemOperations>,
    xdg_directories: Arc<XdgDirectories>,
}

impl TaskManager {
    pub fn new(ctx: &AppContext) -> Result<Self, String> {
        let repo_manager = RepoManager::new(
            ctx.xdg_directories(),
            ctx.file_system(),
            ctx.git_operations(),
        );
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            ctx.file_system(),
            ctx.notification_client(),
        );

        Ok(Self {
            task_runner,
            task_storage: None,
            file_system: ctx.file_system(),
            xdg_directories: ctx.xdg_directories(),
        })
    }

    pub fn with_storage(ctx: &AppContext) -> Result<Self, String> {
        let repo_manager = RepoManager::new(
            ctx.xdg_directories(),
            ctx.file_system(),
            ctx.git_operations(),
        );
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            ctx.file_system(),
            ctx.notification_client(),
        );

        let _repo_root = find_repository_root(Path::new("."))
            .map_err(|e| format!("Failed to find repository root: {}", e))?;

        Ok(Self {
            task_runner,
            task_storage: Some(get_task_storage(ctx.xdg_directories(), ctx.file_system())),
            file_system: ctx.file_system(),
            xdg_directories: ctx.xdg_directories(),
        })
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
        let task = task.unwrap();
        let repo_hash = crate::storage::get_repo_hash(&task.repo_root);
        let task_dir = self.xdg_directories.task_dir(task_id, &repo_hash);
        if self.file_system.exists(&task_dir).await.unwrap_or(false) {
            self.file_system
                .remove_dir(&task_dir)
                .await
                .map_err(|e| format!("Error deleting task directory: {}", e))?;
        }

        Ok(())
    }

    /// Delete all completed tasks
    pub async fn clean_tasks(&self) -> Result<usize, String> {
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
        for task in &completed_tasks {
            let repo_hash = crate::storage::get_repo_hash(&task.repo_root);
            let task_dir = self.xdg_directories.task_dir(&task.id, &repo_hash);
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

        Ok(deleted_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::FixedResponseDockerClient;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_delete_task() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::context::git_operations::tests::MockGitOperations;
        use std::env;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime");
        env::set_var("XDG_DATA_HOME", test_data_dir.to_string_lossy().to_string());
        env::set_var(
            "XDG_RUNTIME_DIR",
            test_runtime_dir.to_string_lossy().to_string(),
        );

        // Create XdgDirectories instance
        let xdg = Arc::new(XdgDirectories::new().unwrap());

        // Create a task to test with
        let repo_root = crate::repo_utils::find_repository_root(Path::new(".")).unwrap();
        let task_id = "test-task-123".to_string();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let _task = Task::new_with_id(
            task_id.clone(),
            repo_root.clone(),
            "test-task".to_string(),
            "feature".to_string(),
            Some("Test description".to_string()),
            None,
            None,
            30,
        );

        // Get XDG paths
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create mock file system with necessary structure
        let git_dir = repo_root.join(".git");

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&task_dir_path.to_string_lossy().to_string())
                .with_file(&format!("{}/test.txt", task_dir_path.to_string_lossy()), "test content")
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &format!(r#"[{{"id":"{}","repo_root":"{}","name":"test-task","task_type":"feature","description":"Test description","instructions_file":null,"agent":null,"timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":null,"error_message":null}}]"#, task_id, repo_root.to_string_lossy()))
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
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
        use std::env;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data2");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime2");
        env::set_var("XDG_DATA_HOME", test_data_dir.to_string_lossy().to_string());
        env::set_var(
            "XDG_RUNTIME_DIR",
            test_runtime_dir.to_string_lossy().to_string(),
        );

        // Create XdgDirectories instance
        let xdg = Arc::new(XdgDirectories::new().unwrap());

        // Create tasks with different statuses
        let repo_root = crate::repo_utils::find_repository_root(Path::new(".")).unwrap();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let queued_task_id = "queued-task-123".to_string();
        let completed_task_id = "completed-task-456".to_string();

        let _queued_task = Task::new_with_id(
            queued_task_id.clone(),
            repo_root.clone(),
            "queued-task".to_string(),
            "feature".to_string(),
            Some("Queued task".to_string()),
            None,
            None,
            30,
        );

        let mut completed_task = Task::new_with_id(
            completed_task_id.clone(),
            repo_root.clone(),
            "completed-task".to_string(),
            "bug-fix".to_string(),
            Some("Completed task".to_string()),
            None,
            None,
            30,
        );
        completed_task.status = TaskStatus::Complete;

        // Get XDG paths
        let queued_dir_path = xdg.task_dir(&queued_task_id, &repo_hash);
        let completed_dir_path = xdg.task_dir(&completed_task_id, &repo_hash);
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create initial tasks.json with both tasks
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"queued-task","task_type":"feature","description":"Queued task","instructions_file":null,"agent":null,"timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":null,"error_message":null}},{{"id":"{}","repo_root":"{}","name":"completed-task","task_type":"bug-fix","description":"Completed task","instructions_file":null,"agent":null,"timeout":30,"status":"COMPLETE","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":"2024-01-01T01:00:00Z","branch_name":null,"error_message":null}}]"#,
            queued_task_id,
            repo_root.to_string_lossy(),
            completed_task_id,
            repo_root.to_string_lossy()
        );

        // Create mock file system with necessary structure
        let git_dir = repo_root.join(".git");

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&queued_dir_path.to_string_lossy().to_string())
                .with_dir(&completed_dir_path.to_string_lossy().to_string())
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let completed_count = result.unwrap();
        assert_eq!(completed_count, 1);

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
        use std::env;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data3");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime3");
        env::set_var("XDG_DATA_HOME", test_data_dir.to_string_lossy().to_string());
        env::set_var(
            "XDG_RUNTIME_DIR",
            test_runtime_dir.to_string_lossy().to_string(),
        );

        // Create XdgDirectories instance
        let xdg = Arc::new(XdgDirectories::new().unwrap());

        // Create a task with a specific ID using new_with_id
        let repo_root = crate::repo_utils::find_repository_root(Path::new(".")).unwrap();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_id = "2024-01-15-1430-test-feature".to_string();
        let mut completed_task = Task::new_with_id(
            task_id.clone(),
            repo_root.clone(),
            "test-feature".to_string(),
            "feature".to_string(),
            Some("Test feature".to_string()),
            None,
            None,
            30,
        );
        completed_task.status = TaskStatus::Complete;

        // Get XDG paths
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"test-feature","task_type":"feature","description":"Test feature","instructions_file":null,"agent":null,"timeout":30,"status":"COMPLETE","created_at":"2024-01-15T14:30:00Z","started_at":null,"completed_at":"2024-01-15T15:00:00Z","branch_name":null,"error_message":null}}]"#,
            task_id,
            repo_root.to_string_lossy()
        );

        // Create mock file system with necessary structure
        let git_dir = repo_root.join(".git");

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&task_dir_path.to_string_lossy().to_string())
                .with_file(
                    &format!("{}/instructions.md", task_dir_path.to_string_lossy()),
                    "Test instructions",
                )
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let git_ops = Arc::new(MockGitOperations::new());

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let completed_count = result.unwrap();
        assert_eq!(completed_count, 1);

        // Verify directory was deleted
        let task_dir_exists = fs.exists(&task_dir_path).await.unwrap();
        assert!(!task_dir_exists, "Task directory should have been deleted");
    }
}
