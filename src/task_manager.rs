use crate::docker::{get_docker_manager, DockerManager};
use crate::git::{get_repo_manager, RepoManager};
use crate::task::{get_task_storage, Task, TaskStatus, TaskStorage};
use std::path::{Path, PathBuf};

pub struct TaskManager {
    repo_manager: RepoManager,
    docker_manager: Box<dyn DockerManagerTrait>,
    task_storage: Option<Box<dyn TaskStorage>>,
}

// Trait for Docker manager to allow testing
#[async_trait::async_trait]
pub trait DockerManagerTrait: Send + Sync {
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
impl<C: crate::docker::DockerClient + Send + Sync> DockerManagerTrait for DockerManager<C> {
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
    pub repo_path: PathBuf,
    pub branch_name: String,
    pub output: String,
}

#[derive(Debug)]
pub struct TaskExecutionError {
    pub message: String,
    pub should_update_task: bool,
    pub task_update: Option<Task>,
}

impl From<String> for TaskExecutionError {
    fn from(message: String) -> Self {
        Self {
            message,
            should_update_task: false,
            task_update: None,
        }
    }
}

impl TaskManager {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            repo_manager: get_repo_manager(),
            docker_manager: Box::new(get_docker_manager()?),
            task_storage: None,
        })
    }

    pub fn with_storage() -> Result<Self, String> {
        Ok(Self {
            repo_manager: get_repo_manager(),
            docker_manager: Box::new(get_docker_manager()?),
            task_storage: Some(get_task_storage().map_err(|e| e.to_string())?),
        })
    }

    #[cfg(test)]
    pub fn with_mocks(
        repo_manager: RepoManager,
        docker_manager: Box<dyn DockerManagerTrait>,
        task_storage: Option<Box<dyn TaskStorage>>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            task_storage,
        }
    }

    /// Prepare instructions file by copying it to the task directory
    pub fn prepare_instructions_file(
        &self,
        instructions_path: &str,
        repo_path: &Path,
    ) -> Result<PathBuf, String> {
        // Read the instructions file to verify it exists
        std::fs::read_to_string(instructions_path)
            .map_err(|e| format!("Error reading instructions file: {}", e))?;

        // Copy instructions file to task folder (parent of repo_path)
        let task_folder = repo_path
            .parent()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let inst_filename = std::path::Path::new(instructions_path)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("instructions.md"));
        let dest_path = task_folder.join(inst_filename);

        std::fs::copy(instructions_path, &dest_path)
            .map_err(|e| format!("Error copying instructions file: {}", e))?;

        println!("Copied instructions file to: {}", dest_path.display());
        Ok(dest_path)
    }

    /// Build command for running the task
    pub fn build_task_command(
        &self,
        description: Option<&String>,
        instructions_file_path: Option<&PathBuf>,
    ) -> Result<Vec<String>, String> {
        if let Some(inst_path) = instructions_file_path {
            // Get just the filename for the container path
            let inst_filename = inst_path
                .file_name()
                .ok_or_else(|| "Invalid instructions file path".to_string())?
                .to_str()
                .ok_or_else(|| "Invalid instructions file name".to_string())?;

            Ok(vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions | jq",
                    inst_filename
                ),
            ])
        } else if let Some(desc) = description {
            Ok(vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "claude -p --verbose --output-format stream-json --dangerously-skip-permissions '{}' | jq",
                    desc
                ),
            ])
        } else {
            Err("Task has neither description nor instructions".to_string())
        }
    }

    /// Execute a task (used by both quick and run commands)
    pub async fn execute_task(
        &self,
        task_name: &str,
        description: Option<&String>,
        instructions_path: Option<&String>,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Copy repository for the task
        let (repo_path, branch_name) = self
            .repo_manager
            .copy_repo(task_name)
            .map_err(|e| format!("Error copying repository: {}", e))?;

        println!("Created repository copy at: {}", repo_path.display());

        // Handle instructions file if provided
        let instructions_file_path = if let Some(inst_path) = instructions_path {
            Some(self.prepare_instructions_file(inst_path, &repo_path)?)
        } else {
            None
        };

        // Build the command
        let command = self.build_task_command(description, instructions_file_path.as_ref())?;

        // Launch Docker container
        println!("Launching Docker container...");
        let output = self
            .docker_manager
            .run_task_container(
                "tsk/base",
                &repo_path,
                command,
                instructions_file_path.as_ref(),
            )
            .await
            .map_err(|e| format!("Error running container: {}", e))?;

        println!("Container execution completed successfully");
        println!("Output:\n{}", output);

        // Commit any changes made by the container
        let commit_message = format!("TSK automated changes for task: {}", task_name);
        if let Err(e) = self
            .repo_manager
            .commit_changes(&repo_path, &commit_message)
        {
            eprintln!("Error committing changes: {}", e);
        }

        // Fetch changes back to main repository
        if let Err(e) = self.repo_manager.fetch_changes(&repo_path, &branch_name) {
            eprintln!("Error fetching changes: {}", e);
        } else {
            println!(
                "Branch {} is now available in the main repository",
                branch_name
            );
        }

        Ok(TaskExecutionResult {
            repo_path,
            branch_name,
            output,
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
        match self
            .execute_task(
                &task.name,
                task.description.as_ref(),
                task.instructions_file.as_ref(),
            )
            .await
        {
            Ok(result) => {
                // Update task status to complete if we have storage
                if let Some(ref storage) = self.task_storage {
                    let mut complete_task = task.clone();
                    complete_task.status = TaskStatus::Complete;
                    complete_task.completed_at = Some(chrono::Utc::now());
                    complete_task.branch_name = Some(result.branch_name.clone());

                    if let Err(e) = storage.update_task(complete_task).await {
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
        let (repo_path, branch_name) = self.repo_manager.copy_repo(task_name)?;

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
        {
            eprintln!("Error committing changes: {}", e);
        }

        // Fetch changes back to main repository
        if let Err(e) = self.repo_manager.fetch_changes(&repo_path, &branch_name) {
            eprintln!("Error fetching changes: {}", e);
        } else {
            println!(
                "Branch {} is now available in the main repository",
                branch_name
            );
        }

        Ok(())
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
        use crate::git::{CommandExecutor, RepoManager};
        use std::os::unix::process::ExitStatusExt;
        use std::process::{ExitStatus, Output};

        struct MockCommandExecutor;

        impl CommandExecutor for MockCommandExecutor {
            fn execute(&self, program: &str, args: &[&str]) -> Result<Output, String> {
                if program == "git" {
                    match args.get(0) {
                        Some(&"rev-parse") => Ok(Output {
                            status: ExitStatus::from_raw(0),
                            stdout: b".git\n".to_vec(),
                            stderr: Vec::new(),
                        }),
                        Some(&"-C") if args.len() > 2 && args[2] == "checkout" => Ok(Output {
                            status: ExitStatus::from_raw(0),
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                        }),
                        Some(&"-C") if args.len() > 2 && args[2] == "status" => Ok(Output {
                            status: ExitStatus::from_raw(0),
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                        }),
                        _ => Ok(Output {
                            status: ExitStatus::from_raw(0),
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                        }),
                    }
                } else {
                    Err(format!("Unknown command: {}", program))
                }
            }
        }

        RepoManager::with_executor(Box::new(MockCommandExecutor))
    }

    #[tokio::test]
    async fn test_build_task_command_with_description() {
        let temp_dir = TempDir::new().unwrap();
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None);

        let description = Some("Test task description".to_string());
        let command = task_manager
            .build_task_command(description.as_ref(), None)
            .unwrap();

        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("Test task description"));
        assert!(command[2].contains("claude -p"));
    }

    #[tokio::test]
    async fn test_build_task_command_with_instructions() {
        let temp_dir = TempDir::new().unwrap();
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None);

        let inst_path = PathBuf::from("/tmp/instructions.md");
        let command = task_manager
            .build_task_command(None, Some(&inst_path))
            .unwrap();

        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p"));
    }

    #[tokio::test]
    async fn test_build_task_command_no_input() {
        let temp_dir = TempDir::new().unwrap();
        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None);

        let result = task_manager.build_task_command(None, None);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Task has neither description nor instructions"
        );
    }

    #[tokio::test]
    async fn test_execute_task_success() {
        let temp_dir = TempDir::new().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create a test git directory and .tsk directory structure
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".tsk/tasks")).unwrap();

        // Create a dummy file to be copied
        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();

        let repo_manager = create_mock_repo_manager(&temp_dir);
        let docker_manager = Box::new(MockDockerManager::new());
        let task_manager = TaskManager::with_mocks(repo_manager, docker_manager, None);

        let description = Some("Test task".to_string());
        let result = task_manager
            .execute_task("test-task", description.as_ref(), None)
            .await;

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));

        // The test passes if no error was thrown
    }
}
