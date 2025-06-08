use crate::context::file_system::FileSystemOperations;
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::log_processor::LogProcessor;
use crate::task::Task;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

pub struct TaskRunner {
    repo_manager: RepoManager,
    docker_manager: DockerManager,
    file_system: Arc<dyn FileSystemOperations>,
}

impl TaskRunner {
    pub fn new(
        repo_manager: RepoManager,
        docker_manager: DockerManager,
        file_system: Arc<dyn FileSystemOperations>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            file_system,
        }
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

    /// Execute a task
    pub async fn execute_task(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Copy repository for the task
        let (repo_path, branch_name) = self
            .repo_manager
            .copy_repo(&task.id)
            .await
            .map_err(|e| format!("Error copying repository: {}", e))?;

        println!("Created repository copy at: {}", repo_path.display());

        // Ensure we have an instructions file
        let instructions_file_path = match &task.instructions_file {
            Some(path) => PathBuf::from(path),
            None => return Err("No instructions file provided".to_string().into()),
        };

        // Build the command
        let command = self.build_task_command(&instructions_file_path)?;

        // Create a log processor with file system
        let mut log_processor = LogProcessor::with_file_system(self.file_system.clone());

        // Launch Docker container with streaming
        println!("Launching Docker container...");
        println!("\n{}", "=".repeat(60));

        let output = {
            // Use streaming version
            self.docker_manager
                .run_task_container(
                    "tsk/base",
                    &repo_path,
                    command.clone(),
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
        };

        println!("\n{}", "=".repeat(60));
        println!("Container execution completed successfully");

        // Save the full log file
        let task_dir = repo_path.parent().unwrap_or(&repo_path);
        let log_file_path = task_dir.join(format!("{}-full.log", task.name));
        if let Err(e) = log_processor.save_full_log(&log_file_path).await {
            eprintln!("Warning: Failed to save full log file: {}", e);
        } else {
            println!("Full log saved to: {}", log_file_path.display());
        }

        // Commit any changes made by the container
        let commit_message = format!("TSK automated changes for task: {}", task.name);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::docker_client::DockerClient;
    use crate::task::{Task, TaskStatus};
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
    async fn test_build_task_command_with_instructions() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::git::RepoManager;

        let fs = Arc::new(MockFileSystem::new());
        let git_ops = Arc::new(crate::context::git_operations::tests::MockGitOperations::new());
        let docker_client = Arc::new(MockDockerClient::new());

        let repo_manager = RepoManager::new(fs.clone(), git_ops);
        let docker_manager = crate::docker::DockerManager::new(docker_client);
        let task_runner = TaskRunner::new(repo_manager, docker_manager, fs);

        let inst_path = PathBuf::from("/tmp/instructions.md");
        let command = task_runner.build_task_command(&inst_path).unwrap();

        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p"));
    }

    #[tokio::test]
    async fn test_execute_task_success() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::git::RepoManager;

        // Create mock file system with necessary files and directories
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(".git")
                .with_dir(".tsk")
                .with_dir(".tsk/tasks")
                .with_file("test.txt", "test content")
                .with_file("instructions.md", "Test task instructions"),
        );

        let git_ops = Arc::new(crate::context::git_operations::tests::MockGitOperations::new());
        let docker_client = Arc::new(MockDockerClient::new());

        let repo_manager = RepoManager::new(fs.clone(), git_ops);
        let docker_manager = crate::docker::DockerManager::new(docker_client);
        let task_runner = TaskRunner::new(repo_manager, docker_manager, fs);

        let task = Task {
            id: "test-task-123".to_string(),
            name: "test-task".to_string(),
            task_type: "feature".to_string(),
            description: Some("Test description".to_string()),
            instructions_file: Some("instructions.md".to_string()),
            agent: None,
            timeout: 30,
            status: TaskStatus::Queued,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            branch_name: None,
            error_message: None,
        };

        let result = task_runner.execute_task(&task).await;

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));
    }
}
