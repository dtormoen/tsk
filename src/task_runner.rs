use crate::agent::AgentProvider;
use crate::context::file_system::FileSystemOperations;
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::task::Task;
use std::path::PathBuf;
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

    /// Execute a task
    pub async fn execute_task(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Get the agent for this task
        let agent_name = task
            .agent
            .as_deref()
            .unwrap_or(AgentProvider::default_agent());
        let agent = AgentProvider::get_agent(agent_name)
            .map_err(|e| format!("Error getting agent: {}", e))?;

        // Validate the agent
        agent
            .validate()
            .await
            .map_err(|e| format!("Agent validation failed: {}", e))?;

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

        // Build the command using the agent
        let command =
            agent.build_command(instructions_file_path.to_str().unwrap_or("instructions.md"));

        // Create a log processor for this agent
        let mut log_processor = agent.create_log_processor(self.file_system.clone());

        // Launch Docker container with streaming
        println!("Launching Docker container with {} agent...", agent.name());
        println!("\n{}", "=".repeat(60));

        let output = {
            // Use streaming version
            self.docker_manager
                .run_task_container(
                    agent.docker_image(),
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
    pub async fn run_debug_container(
        &self,
        task_name: &str,
        agent_name: Option<&str>,
    ) -> Result<(), String> {
        // Get the agent
        let agent_name = agent_name.unwrap_or(AgentProvider::default_agent());
        let agent = AgentProvider::get_agent(agent_name)
            .map_err(|e| format!("Error getting agent: {}", e))?;

        // Validate the agent
        agent.validate().await?;

        // Copy repository for the debug session
        let (repo_path, branch_name) = self.repo_manager.copy_repo(task_name).await?;

        println!(
            "Successfully created repository copy at: {}",
            repo_path.display()
        );

        // Launch Docker container
        println!("Launching Docker container with {} agent...", agent.name());

        let container_name = self
            .docker_manager
            .create_debug_container(agent.docker_image(), &repo_path, agent.as_ref())
            .await?;

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
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::FixedResponseDockerClient;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_execute_task_success() {
        use crate::context::file_system::tests::MockFileSystem;
        use crate::git::RepoManager;

        // Set up a temporary home directory with a mock .claude.json file
        let temp_dir = tempfile::tempdir().unwrap();
        let claude_json_path = temp_dir.path().join(".claude.json");
        std::fs::write(&claude_json_path, "{}").unwrap();
        std::env::set_var("HOME", temp_dir.path());

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
