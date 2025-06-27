use crate::agent::AgentProvider;
use crate::context::file_system::FileSystemOperations;
use crate::docker::{DockerManager, image_manager::DockerImageManager};
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
    notification_client: Arc<dyn NotificationClient>,
}

impl TaskRunner {
    pub fn new(
        repo_manager: RepoManager,
        docker_manager: DockerManager,
        docker_image_manager: Arc<DockerImageManager>,
        _file_system: Arc<dyn FileSystemOperations>, // Keep for compatibility
        notification_client: Arc<dyn NotificationClient>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            docker_image_manager,
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
            .map_err(|e| format!("Error getting agent: {e}"))?;

        // Validate the agent
        agent
            .validate()
            .await
            .map_err(|e| format!("Agent validation failed: {e}"))?;

        // Run agent warmup
        agent
            .warmup()
            .await
            .map_err(|e| format!("Agent warmup failed: {e}"))?;

        // Use the pre-copied repository path
        let repo_path = task.copied_repo_path.clone();

        let branch_name = task.branch_name.clone();

        println!("Using repository copy at: {}", repo_path.display());

        // Get the instructions file path
        let instructions_file_path = PathBuf::from(&task.instructions_file);

        // Launch Docker container
        println!("Launching Docker container with {} agent...", agent.name());
        println!("\n{}", "=".repeat(60));

        let (output, task_result_from_container) = {
            // Ensure the Docker image exists - always rebuild to pick up any changes
            let docker_image = self
                .docker_image_manager
                .ensure_image(
                    &task.tech_stack,
                    &task.agent,
                    Some(&task.project),
                    Some(&repo_path),
                    true,
                )
                .await
                .map_err(|e| format!("Error ensuring Docker image: {e}"))?;

            if docker_image.used_fallback {
                println!(
                    "Note: Using default project layer as project-specific layer was not found"
                );
            }

            // Prepare log file path for non-interactive sessions
            let log_file_path = if !is_interactive {
                let task_dir = repo_path.parent().unwrap_or(&repo_path);
                Some(task_dir.join(format!("{}-full.log", task.name)))
            } else {
                None
            };

            // Run the container using the unified method
            self.docker_manager
                .run_task_container(
                    &docker_image.tag,
                    &repo_path,
                    Some(&instructions_file_path),
                    agent.as_ref(),
                    is_interactive,
                    &task.name,
                    log_file_path.as_deref(),
                )
                .await
                .map_err(|e| format!("Error running container: {e}"))?
        };

        println!("\n{}", "=".repeat(60));
        println!("Container execution completed successfully");

        // Commit any changes made by the container
        let commit_message = format!("TSK automated changes for task: {}", task.name);
        if let Err(e) = self
            .repo_manager
            .commit_changes(&repo_path, &commit_message)
            .await
        {
            eprintln!("Error committing changes: {e}");
        }

        // Fetch changes back to main repository
        match self
            .repo_manager
            .fetch_changes(&repo_path, &branch_name, &task.repo_root)
            .await
        {
            Ok(true) => {
                println!("Branch {branch_name} is now available in the main repository");
            }
            Ok(false) => {
                println!("No changes to merge - branch was not created");
            }
            Err(e) => {
                eprintln!("Error fetching changes: {e}");
            }
        }

        // Use the task result from the container execution
        let task_result = task_result_from_container;

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
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
        }

        // Set up git configuration for tests
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", temp_dir.path().join(".gitconfig"));
        }
        unsafe {
            std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        }

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
        unsafe {
            std::env::set_var("XDG_DATA_HOME", "/tmp/test-xdg-data");
        }
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/tmp/test-xdg-runtime");
        }
        let xdg_directories = Arc::new(crate::storage::XdgDirectories::new().unwrap());

        let repo_manager = RepoManager::new(xdg_directories.clone(), fs.clone(), git_ops);
        let docker_manager = crate::docker::DockerManager::new(docker_client.clone(), fs.clone());

        // Create a mock docker image manager
        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::docker::composer::DockerComposer;
        use crate::docker::template_manager::DockerTemplateManager;

        let template_manager =
            DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), xdg_directories.clone());
        let composer = DockerComposer::new(DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            xdg_directories,
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
            repo_root: temp_dir.path().to_path_buf(),
            name: "test-task".to_string(),
            task_type: "feature".to_string(),
            instructions_file: "instructions.md".to_string(),
            agent: "claude-code".to_string(),
            timeout: 30,
            status: TaskStatus::Queued,
            created_at: chrono::Local::now(),
            started_at: None,
            completed_at: None,
            branch_name: "tsk/test-task-123".to_string(),
            error_message: None,
            source_commit: "abc123".to_string(),
            tech_stack: "default".to_string(),
            project: "default".to_string(),
            copied_repo_path: std::env::current_dir().unwrap(),
        };

        let result = task_runner.execute_task(&task, false).await;

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));
    }
}
