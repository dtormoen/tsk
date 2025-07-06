use crate::assets::layered::LayeredAssetManager;
use crate::context::{AppContext, file_system::FileSystemOperations};
use crate::docker::DockerManager;
use crate::docker::composer::DockerComposer;
use crate::docker::image_manager::DockerImageManager;
use crate::docker::template_manager::DockerTemplateManager;
use crate::git::RepoManager;
use crate::repo_utils::find_repository_root;
use crate::storage::XdgDirectories;
use crate::task::{Task, TaskBuilder, TaskStatus};
use crate::task_runner::{TaskExecutionError, TaskExecutionResult, TaskRunner};
use crate::task_storage::{TaskStorage, get_task_storage};
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
        let docker_manager = DockerManager::new(ctx.docker_client(), ctx.file_system());

        // Create image manager with a default configuration
        // Individual tasks will create their own image managers with task-specific repos
        let project_root = find_repository_root(std::path::Path::new(".")).ok();
        let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
            project_root.as_deref(),
            &ctx.xdg_directories(),
        ));
        let template_manager =
            DockerTemplateManager::new(asset_manager.clone(), ctx.xdg_directories());
        let composer = DockerComposer::new(DockerTemplateManager::new(
            asset_manager,
            ctx.xdg_directories(),
        ));
        let image_manager = Arc::new(DockerImageManager::new(
            ctx.docker_client(),
            template_manager,
            composer,
        ));

        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            image_manager,
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
        let docker_manager = DockerManager::new(ctx.docker_client(), ctx.file_system());

        // Create image manager with a default configuration
        // Individual tasks will create their own image managers with task-specific repos
        let project_root = find_repository_root(std::path::Path::new(".")).ok();
        let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
            project_root.as_deref(),
            &ctx.xdg_directories(),
        ));
        let template_manager =
            DockerTemplateManager::new(asset_manager.clone(), ctx.xdg_directories());
        let composer = DockerComposer::new(DockerTemplateManager::new(
            asset_manager,
            ctx.xdg_directories(),
        ));
        let image_manager = Arc::new(DockerImageManager::new(
            ctx.docker_client(),
            template_manager,
            composer,
        ));

        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            image_manager,
            ctx.file_system(),
            ctx.notification_client(),
        );

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
                eprintln!("Error updating task status: {e}");
            }
        }

        // Execute the task
        let execution_result = self.task_runner.execute_task(task, false).await;

        match execution_result {
            Ok(result) => {
                // Update task status based on the task result if we have storage
                if let Some(ref storage) = self.task_storage {
                    let mut updated_task = task.clone();
                    updated_task.completed_at = Some(chrono::Utc::now());
                    updated_task.branch_name = result.branch_name.clone();

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
                        eprintln!("Error updating task status: {e}");
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
                        eprintln!("Error updating task status: {storage_err}");
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
            .map_err(|e| format!("Error getting task: {e}"))?;

        if task.is_none() {
            return Err(format!("Task with ID '{task_id}' not found"));
        }

        // Delete from storage first
        storage
            .delete_task(task_id)
            .await
            .map_err(|e| format!("Error deleting task from storage: {e}"))?;

        // Delete the task directory
        let task = task.unwrap();
        let repo_hash = crate::storage::get_repo_hash(&task.repo_root);
        let task_dir = self.xdg_directories.task_dir(task_id, &repo_hash);
        if self.file_system.exists(&task_dir).await.unwrap_or(false) {
            self.file_system
                .remove_dir(&task_dir)
                .await
                .map_err(|e| format!("Error deleting task directory: {e}"))?;
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
            .map_err(|e| format!("Error listing tasks: {e}"))?;

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
            .map_err(|e| format!("Error deleting completed tasks: {e}"))?;

        Ok(deleted_count)
    }

    /// Retry a task by creating a new task with the same instructions
    pub async fn retry_task(
        &self,
        task_id: &str,
        edit_instructions: bool,
        ctx: &AppContext,
    ) -> Result<String, String> {
        // Get task storage
        let storage = match &self.task_storage {
            Some(s) => s,
            None => return Err("Task storage not initialized".to_string()),
        };

        // Retrieve the original task
        let original_task = storage
            .get_task(task_id)
            .await
            .map_err(|e| format!("Error getting task: {e}"))?;

        let original_task = match original_task {
            Some(task) => task,
            None => return Err(format!("Task with ID '{task_id}' not found")),
        };

        // Validate that the task has been executed (not Queued)
        if original_task.status == TaskStatus::Queued {
            return Err("Cannot retry a task that hasn't been executed yet".to_string());
        }

        // Create a new task name with format: retry-{original_name}
        let new_task_name = format!("retry-{}", original_task.name);

        // Use TaskBuilder to create the new task, leveraging from_existing
        let mut builder = TaskBuilder::from_existing(&original_task);

        builder = builder.name(new_task_name).edit(edit_instructions);

        let new_task = builder
            .build(ctx)
            .await
            .map_err(|e| format!("Failed to build retry task: {e}"))?;

        // Store the new task
        storage
            .add_task(new_task.clone())
            .await
            .map_err(|e| format!("Error adding retry task to storage: {e}"))?;

        Ok(new_task.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::FixedResponseDockerClient;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper function to create a temporary git repository
    fn create_temp_git_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path().to_path_buf();

        // Create .git directory to make it a valid git repo
        std::fs::create_dir(repo_root.join(".git")).unwrap();

        (temp_dir, repo_root)
    }

    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_delete_task() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime");
        let test_config_dir = temp_dir.join("tsk-test-config");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a task to test with
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let task_id = "test-task-123".to_string();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

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
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &format!(r#"[{{"id":"{}","repo_root":"{}","name":"test-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#, task_id, repo_root.to_string_lossy(), task_id, task_dir_path.to_string_lossy()))
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

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
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_clean_tasks() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data2");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime2");
        let test_config_dir = temp_dir.join("tsk-test-config2");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create tasks with different statuses
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let queued_task_id = "queued-task-123".to_string();
        let completed_task_id = "completed-task-456".to_string();

        let _queued_task = Task::new(
            queued_task_id.clone(),
            repo_root.clone(),
            "queued-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            format!("tsk/{queued_task_id}"),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            repo_root.clone(),
        );

        let mut completed_task = Task::new(
            completed_task_id.clone(),
            repo_root.clone(),
            "completed-task".to_string(),
            "fix".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            format!("tsk/{completed_task_id}"),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            repo_root.clone(),
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
            r#"[{{"id":"{}","repo_root":"{}","name":"queued-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}},{{"id":"{}","repo_root":"{}","name":"completed-task","task_type":"fix","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"COMPLETE","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":"2024-01-01T01:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            queued_task_id,
            repo_root.to_string_lossy(),
            queued_task_id,
            queued_dir_path.to_string_lossy(),
            completed_task_id,
            repo_root.to_string_lossy(),
            completed_task_id,
            completed_dir_path.to_string_lossy()
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
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

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
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_retry_task() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data-retry");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime-retry");
        let test_config_dir = temp_dir.join("tsk-test-config-retry");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a completed task to retry
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_id = "2024-01-01-1200-generic-original-task".to_string();
        let mut completed_task = Task::new(
            task_id.clone(),
            repo_root.clone(),
            "original-task".to_string(),
            "generic".to_string(),
            format!(
                "{}/tasks/{}/{}/instructions.md",
                test_data_dir.to_string_lossy(),
                repo_hash,
                task_id
            ),
            "claude-code".to_string(),
            45,
            format!("tsk/{task_id}"),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            repo_root.clone(),
        );
        completed_task.status = TaskStatus::Complete;

        // Get XDG paths
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);
        let instructions_path = task_dir_path.join("instructions.md");
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"original-task","task_type":"generic","instructions_file":"{}","agent":"claude-code","timeout":45,"status":"COMPLETE","created_at":"2024-01-01T12:00:00Z","started_at":"2024-01-01T12:30:00Z","completed_at":"2024-01-01T13:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            instructions_path.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
        );

        // Create mock file system with necessary structure
        let git_dir = repo_root.join(".git");
        let instructions_content =
            "# Original Task Instructions\n\nThis is the original task content.";

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_dir(&task_dir_path.to_string_lossy().to_string())
                .with_file(
                    &instructions_path.to_string_lossy().to_string(),
                    instructions_content,
                )
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg.clone())
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Retry the task
        let result = task_manager.retry_task(&task_id, false, &ctx).await;
        assert!(result.is_ok(), "Failed to retry task: {:?}", result);
        let new_task_id = result.unwrap();

        // Verify new task ID format
        assert!(new_task_id.contains("generic-retry-original-task"));

        // Verify task was added to storage
        let storage = get_task_storage(xdg.clone(), fs.clone());
        let new_task = storage.get_task(&new_task_id).await.unwrap();
        assert!(new_task.is_some());
        let new_task = new_task.unwrap();
        assert_eq!(new_task.name, "retry-original-task");
        assert_eq!(new_task.task_type, "generic");
        assert_eq!(new_task.agent, "claude-code".to_string());
        assert_eq!(new_task.timeout, 45);
        assert_eq!(new_task.status, TaskStatus::Queued);

        // Verify instructions file was created
        let new_task_dir = xdg.task_dir(&new_task_id, &repo_hash);
        let new_instructions_path = new_task_dir.join("instructions.md");
        assert!(fs.exists(&new_instructions_path).await.unwrap());

        // Verify instructions content was copied
        let copied_content = fs.read_file(&new_instructions_path).await.unwrap();
        assert_eq!(copied_content, instructions_content);
    }

    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_retry_task_not_found() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data-retry-notfound");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime-retry-notfound");
        let test_config_dir = temp_dir.join("tsk-test-config-retry-notfound");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Get XDG paths
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create empty tasks.json
        let tasks_json = "[]";

        // Create mock file system with necessary structure
        let git_dir = repo_root.join(".git");

        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&git_dir.to_string_lossy().to_string())
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_file(&tasks_json_path.to_string_lossy().to_string(), tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Try to retry a non-existent task
        let result = task_manager
            .retry_task("non-existent-task", false, &ctx)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Task with ID 'non-existent-task' not found")
        );
    }

    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_retry_task_queued_error() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data-retry-queued");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime-retry-queued");
        let test_config_dir = temp_dir.join("tsk-test-config-retry-queued");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a queued task (should not be retryable)
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let task_id = "2024-01-01-1200-feat-queued-task".to_string();

        // Get XDG paths
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create tasks.json with the queued task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"queued-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T12:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
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
                .with_file(&tasks_json_path.to_string_lossy().to_string(), &tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Try to retry a queued task
        let result = task_manager.retry_task(&task_id, false, &ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot retry a task that hasn't been executed yet")
        );
    }

    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_clean_tasks_with_id_matching() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data3");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime3");
        let test_config_dir = temp_dir.join("tsk-test-config3");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a task with a specific ID using new_with_id
        let (_temp_repo, repo_root) = create_temp_git_repo();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_id = "2024-01-15-1430-feat-test-feature".to_string();
        let mut completed_task = Task::new(
            task_id.clone(),
            repo_root.clone(),
            "test-feature".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            format!("tsk/{task_id}"),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            repo_root.clone(),
        );
        completed_task.status = TaskStatus::Complete;

        // Get XDG paths
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"test-feature","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"COMPLETE","created_at":"2024-01-15T14:30:00Z","started_at":null,"completed_at":"2024-01-15T15:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
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
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

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

    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_with_storage_no_git_repo() {
        use crate::context::file_system::tests::MockFileSystem;

        // Set up XDG environment variables for testing
        let temp_dir = std::env::temp_dir();
        let test_data_dir = temp_dir.join("tsk-test-data-no-git");
        let test_runtime_dir = temp_dir.join("tsk-test-runtime-no-git");
        let test_config_dir = temp_dir.join("tsk-test-config-no-git");

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            test_data_dir.clone(),
            test_runtime_dir,
            test_config_dir,
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Get XDG paths
        let tasks_json_path = xdg.tasks_file();
        let data_dir = xdg.data_dir().to_path_buf();
        let tasks_dir = data_dir.join("tasks");

        // Create empty tasks.json
        let tasks_json = "[]";

        // Create mock file system WITHOUT a .git directory
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&data_dir.to_string_lossy().to_string())
                .with_dir(&tasks_dir.to_string_lossy().to_string())
                .with_file(&tasks_json_path.to_string_lossy().to_string(), tasks_json),
        );

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        // TODO: Replace with real git operations
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);

        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(fs.clone())
            .with_git_operations(git_ops)
            .with_xdg_directories(xdg)
            .build();

        // This should succeed even without being in a git repository
        let result = TaskManager::with_storage(&ctx);
        assert!(
            result.is_ok(),
            "TaskManager::with_storage should work without a git repository"
        );
    }
}
