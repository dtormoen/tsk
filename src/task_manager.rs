use crate::context::{file_system::FileSystemOperations, AppContext};
use crate::docker::DockerManager;
use crate::git::{get_repo_manager, RepoManager};
use crate::log_processor::LogProcessor;
use crate::task::{get_task_storage, Task, TaskStatus, TaskStorage};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct TaskManager {
    repo_manager: RepoManager,
    docker_manager: Box<dyn DockerManagerTrait>,
    task_storage: Option<Box<dyn TaskStorage>>,
    file_system: Arc<dyn FileSystemOperations>,
}

// Trait for Docker manager to allow testing
#[async_trait::async_trait]
pub trait DockerManagerTrait: Send + Sync {
    #[allow(dead_code)] // Used for downcasting in streaming mode
    fn as_any(&self) -> &dyn std::any::Any;

    async fn run_task_container(
        &self,
        image: &str,
        worktree_path: &Path,
        command: Vec<String>,
        instructions_file_path: Option<&PathBuf>,
    ) -> Result<String, String>;

    async fn create_debug_container(
        &self,
        image: &str,
        worktree_path: &Path,
    ) -> Result<String, String>;

    async fn stop_and_remove_container(&self, container_name: &str) -> Result<(), String>;
}

// Implement the trait for the real DockerManager
#[async_trait::async_trait]
impl DockerManagerTrait for DockerManager {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn run_task_container(
        &self,
        image: &str,
        worktree_path: &Path,
        command: Vec<String>,
        instructions_file_path: Option<&PathBuf>,
    ) -> Result<String, String> {
        self.run_task_container(image, worktree_path, command, instructions_file_path)
            .await
    }

    async fn create_debug_container(
        &self,
        image: &str,
        worktree_path: &Path,
    ) -> Result<String, String> {
        self.create_debug_container(image, worktree_path).await
    }

    async fn stop_and_remove_container(&self, container_name: &str) -> Result<(), String> {
        self.stop_and_remove_container(container_name).await
    }
}

pub struct TaskExecutionResult {
    #[allow(dead_code)] // Available for future use by callers
    pub repo_path: PathBuf,
    pub branch_name: String,
    #[allow(dead_code)] // Available for future use by callers
    pub output: String,
    pub task_result: Option<crate::log_processor::TaskResult>,
}

#[derive(Debug)]
pub struct TaskExecutionError {
    pub message: String,
}

impl From<String> for TaskExecutionError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl TaskManager {
    pub fn new(ctx: &AppContext) -> Result<Self, String> {
        Ok(Self {
            repo_manager: get_repo_manager(ctx.file_system(), ctx.git_operations()),
            docker_manager: Box::new(DockerManager::new(ctx.docker_client())),
            task_storage: None,
            file_system: ctx.file_system(),
        })
    }

    pub fn with_storage(ctx: &AppContext) -> Result<Self, String> {
        Ok(Self {
            repo_manager: get_repo_manager(ctx.file_system(), ctx.git_operations()),
            docker_manager: Box::new(DockerManager::new(ctx.docker_client())),
            task_storage: Some(get_task_storage(ctx.file_system())),
            file_system: ctx.file_system(),
        })
    }

    #[cfg(test)]
    pub fn with_mocks(
        repo_manager: RepoManager,
        docker_manager: Box<dyn DockerManagerTrait>,
        task_storage: Option<Box<dyn TaskStorage>>,
        file_system: Arc<dyn FileSystemOperations>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            task_storage,
            file_system,
        }
    }

    /// Prepare instructions file by copying it to the task directory
    pub async fn prepare_instructions_file(
        &self,
        instructions_path: &str,
        repo_path: &Path,
    ) -> Result<PathBuf, String> {
        // Read the instructions file to verify it exists
        self.file_system
            .read_file(Path::new(instructions_path))
            .await
            .map_err(|e| format!("Error reading instructions file: {}", e))?;

        // Copy instructions file to task folder (parent of repo_path)
        let task_folder = repo_path
            .parent()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let inst_filename = std::path::Path::new(instructions_path)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("instructions.md"));
        let dest_path = task_folder.join(inst_filename);

        self.file_system
            .copy_file(Path::new(instructions_path), &dest_path)
            .await
            .map_err(|e| format!("Error copying instructions file: {}", e))?;

        println!("Copied instructions file to: {}", dest_path.display());
        Ok(dest_path)
    }

    /// Build command for running the task
    pub fn build_task_command(&self, instructions_file_path: &Path) -> Result<Vec<String>, String> {
        // Get just the filename for the container path
        let inst_filename = instructions_file_path
            .file_name()
            .ok_or_else(|| "Invalid instructions file path".to_string())?
            .to_str()
            .ok_or_else(|| "Invalid instructions file name".to_string())?;

        Ok(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions",
                inst_filename
            ),
        ])
    }

    /// Execute a task (used by both quick and run commands)
    pub async fn execute_task(
        &self,
        task_name: &str,
        _description: Option<&String>, // Kept for backward compatibility, but always use instructions_path
        instructions_path: Option<&String>,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Copy repository for the task
        let (repo_path, branch_name) = self
            .repo_manager
            .copy_repo(task_name)
            .await
            .map_err(|e| format!("Error copying repository: {}", e))?;

        println!("Created repository copy at: {}", repo_path.display());

        // Ensure we have an instructions file
        let instructions_file_path = if let Some(inst_path) = instructions_path {
            self.prepare_instructions_file(inst_path, &repo_path)
                .await?
        } else {
            return Err("No instructions file provided".to_string().into());
        };

        // Build the command
        let command = self.build_task_command(&instructions_file_path)?;

        // Remove jq from the command since we'll handle formatting ourselves
        let command_without_jq =
            if command.len() >= 3 && command.last() == Some(&"| jq".to_string()) {
                let mut cmd = command.clone();
                if let Some(last) = cmd.last_mut() {
                    *last = last.replace(" | jq", "");
                }
                cmd
            } else {
                command
            };

        // Create a log processor with file system
        let mut log_processor = LogProcessor::with_file_system(self.file_system.clone());

        // Launch Docker container with streaming
        println!("Launching Docker container...");
        println!("\n{}", "=".repeat(60));

        // For streaming, we need to use the concrete docker manager
        #[cfg(not(test))]
        let output = {
            // Try to downcast to the concrete DockerManager type
            if let Some(concrete_manager) =
                self.docker_manager.as_any().downcast_ref::<DockerManager>()
            {
                // Use streaming version
                concrete_manager
                    .run_task_container_with_streaming(
                        "tsk/base",
                        &repo_path,
                        command_without_jq.clone(),
                        Some(&instructions_file_path),
                        |log_line| {
                            // Process each line of output
                            if let Some(formatted) = log_processor.process_line(log_line) {
                                println!("{}", formatted);
                            }
                        },
                    )
                    .await
                    .map_err(|e| format!("Error running container: {}", e))?
            } else {
                // Fallback to non-streaming version
                let output = self
                    .docker_manager
                    .run_task_container(
                        "tsk/base",
                        &repo_path,
                        command_without_jq,
                        Some(&instructions_file_path),
                    )
                    .await
                    .map_err(|e| format!("Error running container: {}", e))?;

                // Process the output all at once
                for line in output.lines() {
                    if let Some(formatted) = log_processor.process_line(line) {
                        println!("{}", formatted);
                    }
                }
                output
            }
        };

        // In test mode, always use the trait version
        #[cfg(test)]
        let output = {
            let output = self
                .docker_manager
                .run_task_container(
                    "tsk/base",
                    &repo_path,
                    command_without_jq,
                    Some(&instructions_file_path),
                )
                .await
                .map_err(|e| format!("Error running container: {}", e))?;

            // Process the output all at once
            for line in output.lines() {
                if let Some(formatted) = log_processor.process_line(line) {
                    println!("{}", formatted);
                }
            }
            output
        };

        println!("\n{}", "=".repeat(60));
        println!("Container execution completed successfully");

        // Save the full log file
        let task_dir = repo_path.parent().unwrap_or(&repo_path);
        let log_file_path = task_dir.join(format!("{}-full.log", task_name));
        if let Err(e) = log_processor.save_full_log(&log_file_path).await {
            eprintln!("Warning: Failed to save full log file: {}", e);
        } else {
            println!("Full log saved to: {}", log_file_path.display());
        }

        // Commit any changes made by the container
        let commit_message = format!("TSK automated changes for task: {}", task_name);
        if let Err(e) = self
            .repo_manager
            .commit_changes(&repo_path, &commit_message)
            .await
        {
            eprintln!("Error committing changes: {}", e);
        }

        // Fetch changes back to main repository
        if let Err(e) = self
            .repo_manager
            .fetch_changes(&repo_path, &branch_name)
            .await
        {
            eprintln!("Error fetching changes: {}", e);
        } else {
            println!(
                "Branch {} is now available in the main repository",
                branch_name
            );
        }

        // Get the final result from the log processor
        let task_result = log_processor.get_final_result().cloned();

        Ok(TaskExecutionResult {
            repo_path,
            branch_name,
            output,
            task_result,
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
        let execution_result = self
            .execute_task(
                &task.name,
                task.description.as_ref(),
                task.instructions_file.as_ref(),
            )
            .await;

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

    /// Run debug container for a task
    pub async fn run_debug_container(&self, task_name: &str) -> Result<(), String> {
        // Copy repository for the debug session
        let (repo_path, branch_name) = self.repo_manager.copy_repo(task_name).await?;

        println!(
            "Successfully created repository copy at: {}",
            repo_path.display()
        );

        // Launch Docker container
        println!("Launching Docker container...");

        let container_name = self
            .docker_manager
            .create_debug_container("tsk/base", &repo_path)
            .await?;

        println!("\nDocker container started successfully!");
        println!("Container name: {}", container_name);
        println!("\nTo connect to the container, run:");
        println!("  docker exec -it {} /bin/bash", container_name);
        println!("\nPress any key to stop the container and exit...");

        // Wait for user input
        use std::io::{self, Read};
        let _ = io::stdin().read(&mut [0u8]).unwrap();

        println!("\nStopping container...");
        self.docker_manager
            .stop_and_remove_container(&container_name)
            .await?;

        println!("Container stopped and removed successfully");

        // Commit any changes made during debug session
        let commit_message = format!("TSK debug session changes for: {}", task_name);
        if let Err(e) = self
            .repo_manager
            .commit_changes(&repo_path, &commit_message)
            .await
        {
            eprintln!("Error committing changes: {}", e);
        }

        // Fetch changes back to main repository
        if let Err(e) = self
            .repo_manager
            .fetch_changes(&repo_path, &branch_name)
            .await
        {
            eprintln!("Error fetching changes: {}", e);
        } else {
            println!(
                "Branch {} is now available in the main repository",
                branch_name
            );
        }

        Ok(())
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
        let task_dir = PathBuf::from(".tsk/tasks").join(task_id);
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
        for task in &completed_tasks {
            let task_dir = PathBuf::from(".tsk/tasks").join(&task.id);
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
        let quick_tasks_dir = PathBuf::from(".tsk/quick-tasks");
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
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    // Mock Docker Manager for testing
    struct MockDockerManager {
        run_task_container_calls: Arc<Mutex<Vec<(String, PathBuf, Vec<String>, Option<PathBuf>)>>>,
        run_task_container_result: Arc<Mutex<Result<String, String>>>,
        create_debug_container_calls: Arc<Mutex<Vec<(String, PathBuf)>>>,
        create_debug_container_result: Arc<Mutex<Result<String, String>>>,
        stop_and_remove_container_calls: Arc<Mutex<Vec<String>>>,
        stop_and_remove_container_result: Arc<Mutex<Result<(), String>>>,
    }

    impl MockDockerManager {
        fn new() -> Self {
            Self {
                run_task_container_calls: Arc::new(Mutex::new(Vec::new())),
                run_task_container_result: Arc::new(Mutex::new(Ok("Test output".to_string()))),
                create_debug_container_calls: Arc::new(Mutex::new(Vec::new())),
                create_debug_container_result: Arc::new(Mutex::new(Ok(
                    "test-debug-container".to_string()
                ))),
                stop_and_remove_container_calls: Arc::new(Mutex::new(Vec::new())),
                stop_and_remove_container_result: Arc::new(Mutex::new(Ok(()))),
            }
        }
    }

    #[async_trait::async_trait]
    impl DockerManagerTrait for MockDockerManager {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        async fn run_task_container(
            &self,
            image: &str,
            worktree_path: &Path,
            command: Vec<String>,
            instructions_file_path: Option<&PathBuf>,
        ) -> Result<String, String> {
            self.run_task_container_calls.lock().await.push((
                image.to_string(),
                worktree_path.to_path_buf(),
                command,
                instructions_file_path.cloned(),
            ));
            self.run_task_container_result.lock().await.clone()
        }

        async fn create_debug_container(
            &self,
            image: &str,
            worktree_path: &Path,
        ) -> Result<String, String> {
            self.create_debug_container_calls
                .lock()
                .await
                .push((image.to_string(), worktree_path.to_path_buf()));
            self.create_debug_container_result.lock().await.clone()
        }

        async fn stop_and_remove_container(&self, container_name: &str) -> Result<(), String> {
            self.stop_and_remove_container_calls
                .lock()
                .await
                .push(container_name.to_string());
            self.stop_and_remove_container_result.lock().await.clone()
        }
    }

    // Mock Repo Manager
    fn create_mock_repo_manager(_temp_dir: &TempDir) -> RepoManager {
        use crate::context::git_operations::tests::MockGitOperations;
        use crate::git::RepoManager;

        use crate::context::file_system::DefaultFileSystem;
        let fs = Arc::new(DefaultFileSystem);
        let git_ops = Arc::new(MockGitOperations::new());
        RepoManager::with_git_operations(fs, git_ops)
    }

    #[tokio::test]
    async fn test_build_task_command_with_instructions() {
        use crate::context::file_system::tests::MockFileSystem;
        let temp_dir = TempDir::new().unwrap();
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let fs = Arc::new(MockFileSystem::new());
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None, fs);

        let inst_path = PathBuf::from("/tmp/instructions.md");
        let command = task_manager.build_task_command(&inst_path).unwrap();

        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p"));
    }

    #[tokio::test]
    #[ignore] // Test passes individually but has race conditions when run with other tests
    async fn test_execute_task_success() {
        let temp_dir = TempDir::new().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create a test git directory and .tsk directory structure
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/tasks")).unwrap();

        // Create a dummy file to be copied
        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();

        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        use crate::context::file_system::DefaultFileSystem;
        let fs = Arc::new(DefaultFileSystem);
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None, fs);

        // Create an instructions file
        let instructions_file = temp_dir.path().join("instructions.md");
        std::fs::write(&instructions_file, "Test task instructions").unwrap();

        let result = task_manager
            .execute_task(
                "test-task",
                None,
                Some(&instructions_file.to_string_lossy().to_string()),
            )
            .await;

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));

        // The test passes if no error was thrown
    }

    #[tokio::test]
    #[ignore] // Test passes individually but has race conditions when run with other tests
    async fn test_delete_task() {
        let temp_dir = TempDir::new().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create a test git directory and .tsk directory structure
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/tasks")).unwrap();

        // Create task storage and add a test task
        use crate::context::file_system::DefaultFileSystem;
        let fs = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs.clone());
        let task = Task::new(
            "test-task".to_string(),
            "feature".to_string(),
            Some("Test description".to_string()),
            None,
            None,
            30,
        );
        let task_id = task.id.clone();
        storage.add_task(task.clone()).await.unwrap();

        // Create task directory
        let task_dir = temp_dir.path().join(".tsk/tasks").join(&task_id);
        std::fs::create_dir_all(&task_dir).unwrap();
        std::fs::write(task_dir.join("test.txt"), "test content").unwrap();

        // Create task manager with storage
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(
            repo_manager,
            docker_manager,
            Some(Box::new(storage)),
            fs.clone(),
        );

        // Delete the task
        let result = task_manager.delete_task(&task_id).await;
        assert!(result.is_ok());

        // Verify task is deleted from storage
        let fs2 = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs2);
        let task_lookup = storage.get_task(&task_id).await.unwrap();
        assert!(task_lookup.is_none());

        // Verify task directory is deleted
        assert!(!task_dir.exists());

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();
    }

    #[tokio::test]
    #[ignore] // Test passes individually but has race conditions when run with other tests
    async fn test_clean_tasks() {
        let temp_dir = TempDir::new().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create a test git directory and .tsk directory structure
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/tasks")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/quick-tasks")).unwrap();

        // Create task storage and add tasks with different statuses
        use crate::context::file_system::DefaultFileSystem;
        let fs2 = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs2.clone());

        // Add queued task
        let queued_task = Task::new(
            "queued-task".to_string(),
            "feature".to_string(),
            Some("Queued task".to_string()),
            None,
            None,
            30,
        );
        storage.add_task(queued_task.clone()).await.unwrap();

        // Add completed task
        let mut completed_task = Task::new(
            "completed-task".to_string(),
            "bug-fix".to_string(),
            Some("Completed task".to_string()),
            None,
            None,
            30,
        );
        completed_task.status = TaskStatus::Complete;
        storage.add_task(completed_task.clone()).await.unwrap();

        // Create task directories
        let queued_dir = temp_dir.path().join(".tsk/tasks").join(&queued_task.id);
        let completed_dir = temp_dir.path().join(".tsk/tasks").join(&completed_task.id);
        std::fs::create_dir_all(&queued_dir).unwrap();
        std::fs::create_dir_all(&completed_dir).unwrap();

        // Create quick task directories
        let quick_task_dir1 = temp_dir
            .path()
            .join(".tsk/quick-tasks/2024-01-01-1200-quick1");
        let quick_task_dir2 = temp_dir
            .path()
            .join(".tsk/quick-tasks/2024-01-01-1300-quick2");
        std::fs::create_dir_all(&quick_task_dir1).unwrap();
        std::fs::create_dir_all(&quick_task_dir2).unwrap();

        // Create task manager with storage
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(
            repo_manager,
            docker_manager,
            Some(Box::new(storage)),
            fs2.clone(),
        );

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok());
        let (completed_count, quick_count) = result.unwrap();
        assert_eq!(completed_count, 1);
        assert_eq!(quick_count, 2);

        // Verify queued task still exists
        let fs2 = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs2);
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Queued);

        // Verify directories are cleaned up
        assert!(queued_dir.exists());
        assert!(!completed_dir.exists());
        assert!(!quick_task_dir1.exists());
        assert!(!quick_task_dir2.exists());

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();
    }

    #[tokio::test]
    #[ignore] // Test passes individually but has race conditions when run with other tests
    async fn test_clean_tasks_with_id_matching() {
        let temp_dir = TempDir::new().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create a test git directory and .tsk directory structure
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/tasks")).unwrap();

        // Create task storage
        use crate::context::file_system::DefaultFileSystem;
        let fs = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs.clone());

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
        storage.add_task(completed_task.clone()).await.unwrap();

        // Create task directory with matching ID
        let task_dir = temp_dir.path().join(".tsk/tasks").join(&task_id);
        std::fs::create_dir_all(&task_dir).unwrap();

        // Create a test file in the directory to ensure the directory is not empty
        std::fs::write(task_dir.join("instructions.md"), "Test instructions").unwrap();

        // Create task manager with storage
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager =
            TaskManager::with_mocks(repo_manager, docker_manager, Some(Box::new(storage)), fs);

        // Clean tasks
        let result = task_manager.clean_tasks().await;
        assert!(result.is_ok());
        let (completed_count, _) = result.unwrap();
        assert_eq!(completed_count, 1);

        // Verify task was removed from storage
        let fs2 = Arc::new(DefaultFileSystem);
        let storage =
            crate::task::JsonTaskStorage::new(temp_dir.path().join(".tsk").as_path(), fs2);
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 0);

        // Verify directory was deleted
        assert!(
            !task_dir.exists(),
            "Task directory should have been deleted"
        );

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();
    }
}
