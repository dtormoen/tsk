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
    use crate::test_utils::TestGitRepository;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper function to set up a test environment with XDG directories, git repository, and AppContext
    async fn setup_test_environment()
    -> anyhow::Result<(Arc<XdgDirectories>, TestGitRepository, AppContext)> {
        // Create temporary directory for XDG
        let temp_dir = TempDir::new()?;

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config))?);
        xdg.ensure_directories()?;

        // Create a test git repository
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;

        // Create AppContext
        let ctx = AppContext::builder()
            .with_xdg_directories(xdg.clone())
            .build();

        Ok((xdg, test_repo, ctx))
    }

    /// Helper function to set up task directory structure with files
    async fn setup_task_directory(
        xdg: &XdgDirectories,
        task_id: &str,
        repo_hash: &str,
        instructions_content: &str,
    ) -> anyhow::Result<std::path::PathBuf> {
        let task_dir_path = xdg.task_dir(task_id, repo_hash);
        std::fs::create_dir_all(&task_dir_path)?;

        let instructions_path = task_dir_path.join("instructions.md");
        std::fs::write(&instructions_path, instructions_content)?;

        Ok(task_dir_path)
    }

    #[tokio::test]
    async fn test_delete_task() {
        use crate::test_utils::TestGitRepository;

        // Create temporary directory for XDG
        let temp_dir = TempDir::new().unwrap();

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let repo_root = test_repo.path().to_path_buf();

        // Create a task
        let task_id = "test-task-123".to_string();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);

        // Create the task directory and file
        std::fs::create_dir_all(&task_dir_path).unwrap();
        std::fs::write(task_dir_path.join("test.txt"), "test content").unwrap();

        // Create tasks.json with the task
        let tasks_json_path = xdg.tasks_file();
        let task_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"test-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
        );
        std::fs::write(&tasks_json_path, task_json).unwrap();

        // Create AppContext
        let ctx = AppContext::builder()
            .with_xdg_directories(xdg.clone())
            .build();

        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        // Delete the task
        let result = task_manager.delete_task(&task_id).await;
        assert!(result.is_ok(), "Failed to delete task: {:?}", result);

        // Verify task directory is deleted
        assert!(
            !task_dir_path.exists(),
            "Task directory should have been deleted"
        );

        // Verify task is removed from storage
        let storage = task_manager.task_storage.as_ref().unwrap();
        let task = storage.get_task(&task_id).await.unwrap();
        assert!(task.is_none(), "Task should have been deleted from storage");
    }

    #[tokio::test]
    async fn test_clean_tasks() {
        // Set up test environment
        let (xdg, test_repo, ctx) = setup_test_environment().await.unwrap();
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create tasks with different statuses
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

        // Create task directories and files
        let queued_dir_path = setup_task_directory(
            &xdg,
            &queued_task_id,
            &repo_hash,
            "Queued task instructions",
        )
        .await
        .unwrap();
        let completed_dir_path = setup_task_directory(
            &xdg,
            &completed_task_id,
            &repo_hash,
            "Completed task instructions",
        )
        .await
        .unwrap();

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

        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, tasks_json).unwrap();

        // Create TaskManager and clean tasks
        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let completed_count = result.unwrap();
        assert_eq!(completed_count, 1);

        // Verify directories are cleaned up
        assert!(
            queued_dir_path.exists(),
            "Queued task directory should still exist"
        );
        assert!(
            !completed_dir_path.exists(),
            "Completed task directory should be deleted"
        );

        // Verify storage was updated
        let storage = task_manager.task_storage.as_ref().unwrap();
        let remaining_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(remaining_tasks.len(), 1);
        assert_eq!(remaining_tasks[0].id, queued_task_id);
    }

    #[tokio::test]
    async fn test_retry_task() {
        // Set up test environment
        let (xdg, test_repo, ctx) = setup_test_environment().await.unwrap();
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create a completed task to retry
        let task_id = "2024-01-01-1200-generic-original-task".to_string();
        let task_dir_path = xdg.task_dir(&task_id, &repo_hash);
        let instructions_path = task_dir_path.join("instructions.md");

        let mut completed_task = Task::new(
            task_id.clone(),
            repo_root.clone(),
            "original-task".to_string(),
            "generic".to_string(),
            instructions_path.to_string_lossy().to_string(),
            "claude-code".to_string(),
            45,
            format!("tsk/{task_id}"),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            task_dir_path.to_path_buf(),
        );
        completed_task.status = TaskStatus::Complete;

        // Set up task directory with instructions
        let instructions_content =
            "# Original Task Instructions\n\nThis is the original task content.";
        setup_task_directory(&xdg, &task_id, &repo_hash, instructions_content)
            .await
            .unwrap();

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"original-task","task_type":"generic","instructions_file":"{}","agent":"claude-code","timeout":45,"status":"COMPLETE","created_at":"2024-01-01T12:00:00Z","started_at":"2024-01-01T12:30:00Z","completed_at":"2024-01-01T13:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            instructions_path.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
        );

        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, tasks_json).unwrap();

        // Create TaskManager and retry the task
        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        let result = task_manager.retry_task(&task_id, false, &ctx).await;
        assert!(result.is_ok(), "Failed to retry task: {:?}", result);
        let new_task_id = result.unwrap();

        // Verify new task ID format
        assert!(new_task_id.contains("generic-retry-original-task"));

        // Verify task was added to storage
        let storage = task_manager.task_storage.as_ref().unwrap();
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
        assert!(new_instructions_path.exists());

        // Verify instructions content was copied
        let copied_content = std::fs::read_to_string(&new_instructions_path).unwrap();
        assert_eq!(copied_content, instructions_content);
    }

    #[tokio::test]
    async fn test_retry_task_not_found() {
        // Set up test environment
        let (xdg, _test_repo, ctx) = setup_test_environment().await.unwrap();

        // Create empty tasks.json
        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, "[]").unwrap();

        // Create TaskManager and try to retry a non-existent task
        let task_manager = TaskManager::with_storage(&ctx).unwrap();

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
    async fn test_retry_task_queued_error() {
        // Set up test environment
        let (xdg, test_repo, ctx) = setup_test_environment().await.unwrap();
        let repo_root = test_repo.path().to_path_buf();

        // Create a queued task (should not be retryable)
        let task_id = "2024-01-01-1200-feat-queued-task".to_string();

        // Create tasks.json with the queued task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"queued-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T12:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            task_id,
            repo_root.to_string_lossy()
        );

        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, tasks_json).unwrap();

        // Create TaskManager and try to retry a queued task
        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        let result = task_manager.retry_task(&task_id, false, &ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot retry a task that hasn't been executed yet")
        );
    }

    #[tokio::test]
    async fn test_clean_tasks_with_id_matching() {
        // Set up test environment
        let (xdg, test_repo, ctx) = setup_test_environment().await.unwrap();
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create a task with a specific ID
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

        // Set up task directory with instructions
        let task_dir_path = setup_task_directory(&xdg, &task_id, &repo_hash, "Test instructions")
            .await
            .unwrap();

        // Create tasks.json with the completed task
        let tasks_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"test-feature","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"COMPLETE","created_at":"2024-01-15T14:30:00Z","started_at":null,"completed_at":"2024-01-15T15:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
        );

        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, tasks_json).unwrap();

        // Create TaskManager and clean tasks
        let task_manager = TaskManager::with_storage(&ctx).unwrap();

        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok(), "Failed to clean tasks: {:?}", result);
        let completed_count = result.unwrap();
        assert_eq!(completed_count, 1);

        // Verify directory was deleted
        assert!(
            !task_dir_path.exists(),
            "Task directory should have been deleted"
        );

        // Verify storage was updated
        let storage = task_manager.task_storage.as_ref().unwrap();
        let remaining_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(remaining_tasks.len(), 0);
    }

    #[tokio::test]
    async fn test_with_storage_no_git_repo() {
        // Create temporary directory for XDG (not a git repo)
        let temp_dir = TempDir::new().unwrap();

        // Create XdgDirectories instance using XdgConfig
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config)).unwrap());
        xdg.ensure_directories().unwrap();

        // Create empty tasks.json
        let tasks_json_path = xdg.tasks_file();
        if let Some(parent) = tasks_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&tasks_json_path, "[]").unwrap();

        // Create AppContext without a git repository
        let ctx = AppContext::builder().with_xdg_directories(xdg).build();

        // This should succeed even without being in a git repository
        let result = TaskManager::with_storage(&ctx);
        assert!(
            result.is_ok(),
            "TaskManager::with_storage should work without a git repository"
        );
    }
}
