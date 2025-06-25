use crate::agent::AgentProvider;
use crate::context::file_system::FileSystemOperations;
use crate::docker::{image_manager::DockerImageManager, DockerManager};
use crate::git::RepoManager;
use crate::notifications::NotificationClient;
use crate::task::Task;
use std::path::PathBuf;
use std::sync::Arc;

pub struct TaskExecutionResult {
    #[allow(dead_code)] // Available for future use by callers
    pub repo_path: PathBuf,
    pub branch_name: String,
    #[allow(dead_code)] // Available for future use by callers
    pub output: String,
    pub task_result: Option<crate::agent::TaskResult>,
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
    docker_image_manager: Arc<DockerImageManager>,
    file_system: Arc<dyn FileSystemOperations>,
    notification_client: Arc<dyn NotificationClient>,
}

impl TaskRunner {
    pub fn new(
        repo_manager: RepoManager,
        docker_manager: DockerManager,
        docker_image_manager: Arc<DockerImageManager>,
        file_system: Arc<dyn FileSystemOperations>,
        notification_client: Arc<dyn NotificationClient>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            docker_image_manager,
            file_system,
            notification_client,
        }
    }

    /// Execute a task
    pub async fn execute_task(
        &self,
        task: &Task,
        is_interactive: bool,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Get the agent for this task
        let agent = AgentProvider::get_agent(&task.agent)
            .map_err(|e| format!("Error getting agent: {}", e))?;

        // Validate the agent
        agent
            .validate()
            .await
            .map_err(|e| format!("Agent validation failed: {}", e))?;

        // Run agent warmup
        agent
            .warmup()
            .await
            .map_err(|e| format!("Agent warmup failed: {}", e))?;

        // Copy repository for the task
        let (repo_path, branch_name) = self
            .repo_manager
            .copy_repo(&task.id, &task.repo_root, Some(&task.source_commit))
            .await
            .map_err(|e| format!("Error copying repository: {}", e))?;

        println!("Created repository copy at: {}", repo_path.display());

        // Get the instructions file path
        let instructions_file_path = PathBuf::from(&task.instructions_file);

        // Build the command using the agent
        let command =
            agent.build_command(instructions_file_path.to_str().unwrap_or("instructions.md"));

        // Create a log processor for this agent
        let mut log_processor = agent.create_log_processor(self.file_system.clone());

        // Launch Docker container with streaming
        println!("Launching Docker container with {} agent...", agent.name());
        println!("\n{}", "=".repeat(60));

        let output = {
            // Ensure the Docker image exists - always rebuild to pick up any changes
            let docker_image = self
                .docker_image_manager
                .ensure_image(&task.tech_stack, &task.agent, Some(&task.project), true)
                .await
                .map_err(|e| format!("Error ensuring Docker image: {}", e))?;

            if docker_image.used_fallback {
                println!(
                    "Note: Using default project layer as project-specific layer was not found"
                );
            }

            if is_interactive {
                // Create interactive container
                let container_name = self
                    .docker_manager
                    .create_task_container_interactive(
                        &docker_image.tag,
                        &repo_path,
                        Some(&instructions_file_path),
                        agent.as_ref(),
                    )
                    .await
                    .map_err(|e| format!("Error creating interactive container: {}", e))?;

                println!("\nDocker container started successfully!");
                println!("Container name: {}", container_name);
                println!("\nStarting interactive session...");

                // Start interactive docker exec session
                let status = std::process::Command::new("docker")
                    .args(["exec", "-it", &container_name, "/bin/bash"])
                    .status()
                    .map_err(|e| format!("Failed to execute docker exec: {}", e))?;

                if !status.success() {
                    eprintln!("Interactive session exited with non-zero status");
                }

                println!("\nStopping container...");
                self.docker_manager
                    .stop_and_remove_container(&container_name)
                    .await
                    .map_err(|e| format!("Error stopping container: {}", e))?;

                println!("Container stopped and removed successfully");

                // Return empty output for interactive sessions
                String::new()
            } else {
                // Use streaming version for non-interactive
                self.docker_manager
                    .run_task_container(
                        &docker_image.tag,
                        &repo_path,
                        command.clone(),
                        Some(&instructions_file_path),
                        agent.as_ref(),
                        |log_line| {
                            // Process each line of output
                            if let Some(formatted) = log_processor.process_line(log_line) {
                                println!("{}", formatted);
                            }
                        },
                    )
                    .await
                    .map_err(|e| format!("Error running container: {}", e))?
            }
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
        match self
            .repo_manager
            .fetch_changes(&repo_path, &branch_name, &task.repo_root)
            .await
        {
            Ok(true) => {
                println!(
                    "Branch {} is now available in the main repository",
                    branch_name
                );
            }
            Ok(false) => {
                println!("No changes to merge - branch was not created");
            }
            Err(e) => {
                eprintln!("Error fetching changes: {}", e);
            }
        }

        // Get the final result from the log processor
        let task_result = log_processor.get_final_result().cloned();

        // Send notification about task completion
        let success = task_result.as_ref().map(|r| r.success).unwrap_or(false);
        let message = task_result.as_ref().map(|r| r.message.as_str());
        self.notification_client
            .notify_task_complete(&task.name, success, message);

        Ok(TaskExecutionResult {
            repo_path,
            branch_name,
            output,
            task_result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::FixedResponseDockerClient;
    use std::sync::Arc;

    #[tokio::test]
    #[ignore = "Test requires Docker to be available for image building"]
    async fn test_execute_task_success() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::git::RepoManager;

        // Set up a temporary home directory with a mock .claude.json file
        let temp_dir = tempfile::tempdir().unwrap();
        let claude_json_path = temp_dir.path().join(".claude.json");
        std::fs::write(&claude_json_path, "{}").unwrap();
        std::env::set_var("HOME", temp_dir.path());

        // Set up git configuration for tests
        std::env::set_var("GIT_CONFIG_GLOBAL", temp_dir.path().join(".gitconfig"));
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");

        // Configure git for the test
        std::process::Command::new("git")
            .args(["config", "--global", "user.name", "Test User"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "--global", "user.email", "test@example.com"])
            .output()
            .unwrap();

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
        let docker_client = Arc::new(FixedResponseDockerClient::default());

        // Create test XDG directories
        std::env::set_var("XDG_DATA_HOME", "/tmp/test-xdg-data");
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/test-xdg-runtime");
        let xdg_directories = Arc::new(crate::storage::XdgDirectories::new().unwrap());

        let repo_manager = RepoManager::new(xdg_directories.clone(), fs.clone(), git_ops);
        let docker_manager = crate::docker::DockerManager::new(docker_client.clone());

        // Create a mock docker image manager
        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::docker::composer::DockerComposer;
        use crate::docker::template_manager::DockerTemplateManager;

        let template_manager = DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            xdg_directories.clone(),
            None,
        );
        let composer = DockerComposer::new(DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            xdg_directories,
            None,
        ));
        let docker_image_manager = Arc::new(DockerImageManager::new(
            docker_client,
            template_manager,
            composer,
        ));

        let notification_client = Arc::new(crate::notifications::NoOpNotificationClient);
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            docker_image_manager,
            fs,
            notification_client,
        );

        let task = Task {
            id: "test-task-123".to_string(),
            repo_root: std::env::current_dir().unwrap(),
            name: "test-task".to_string(),
            task_type: "feature".to_string(),
            instructions_file: "instructions.md".to_string(),
            agent: "claude-code".to_string(),
            timeout: 30,
            status: TaskStatus::Queued,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            branch_name: "tsk/test-task-123".to_string(),
            error_message: None,
            source_commit: "abc123".to_string(),
            tech_stack: "default".to_string(),
            project: "default".to_string(),
        };

        let result = task_runner.execute_task(&task, false).await;

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));
    }
}
