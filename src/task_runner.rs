use crate::agent::AgentProvider;
use crate::context::AppContext;
use crate::docker::{DockerManager, image_manager::DockerImageManager};
use crate::git::RepoManager;
use crate::task::Task;
use std::path::PathBuf;

/// Result of executing a task
///
/// Contains information about the executed task including the repository path,
/// branch name, execution output, and task-specific results. All fields are
/// public for use by callers, including tests and future functionality.
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
    pub is_warmup_failure: bool,
}

impl From<String> for TaskExecutionError {
    fn from(message: String) -> Self {
        Self {
            message,
            is_warmup_failure: false,
        }
    }
}

/// Manages the execution of individual tasks in Docker containers.
///
/// TaskRunner handles the complete lifecycle of task execution including:
/// - Agent validation and warmup
/// - Docker image management and container execution
/// - Repository changes and git operations
/// - Task completion notifications
pub struct TaskRunner {
    ctx: AppContext,
    repo_manager: RepoManager,
    docker_manager: DockerManager,
}

impl TaskRunner {
    /// Creates a new TaskRunner from the application context.
    ///
    /// Extracts all required dependencies from the AppContext and constructs
    /// the necessary components internally.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context providing all required dependencies
    pub fn new(ctx: &AppContext) -> Self {
        let repo_manager = RepoManager::new(ctx);
        let docker_manager = DockerManager::new(ctx.docker_client());

        Self {
            ctx: ctx.clone(),
            repo_manager,
            docker_manager,
        }
    }

    /// Execute a task in a Docker container.
    ///
    /// Performs the complete task execution lifecycle:
    /// 1. Validates and warms up the specified agent
    /// 2. Creates task-specific Docker images with appropriate layers
    /// 3. Runs the container with the task instructions
    /// 4. Commits and fetches changes back to the main repository
    /// 5. Sends completion notifications
    ///
    /// # Arguments
    ///
    /// * `task` - The task to execute
    /// * `is_interactive` - Whether to run the container interactively
    ///
    /// # Returns
    ///
    /// Returns `TaskExecutionResult` on success or `TaskExecutionError` on failure.
    /// Warmup failures are specially marked in the error for retry handling.
    pub async fn execute_task(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Get the agent for this task
        let agent = AgentProvider::get_agent(&task.agent, self.ctx.tsk_config())
            .map_err(|e| format!("Error getting agent: {e}"))?;

        // Validate the agent
        agent
            .validate()
            .await
            .map_err(|e| format!("Agent validation failed: {e}"))?;

        // Run agent warmup
        if let Err(e) = agent.warmup().await {
            return Err(TaskExecutionError {
                message: format!("Agent warmup failed: {e}"),
                is_warmup_failure: true,
            });
        }

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
            // Create a task-specific image manager with the copied repository as the project root
            // This ensures that project-specific dockerfiles are found in the copied repository
            let task_image_manager = DockerImageManager::new(&self.ctx, Some(&repo_path));

            // Ensure the proxy image exists first
            task_image_manager
                .ensure_proxy_image()
                .await
                .map_err(|e| format!("Error ensuring proxy image: {e}"))?;

            // Ensure the Docker image exists - always rebuild to pick up any changes
            let docker_image = task_image_manager
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

            // Run the container using the unified method
            self.docker_manager
                .run_task_container(
                    &docker_image.tag,
                    &repo_path,
                    Some(&instructions_file_path),
                    agent.as_ref(),
                    task.is_interactive,
                    &task.id,
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
        self.ctx
            .notification_client()
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
    use crate::test_utils::git_test_utils::TestGitRepository;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_execute_task_success() {
        use crate::context::AppContext;
        use crate::test_utils::FixedResponseDockerClient;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create necessary files in the repository
        test_repo
            .create_file(".tsk/tasks/instructions.md", "Test task instructions")
            .unwrap();
        test_repo.create_file("test.txt", "test content").unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add test files").unwrap();

        // Create a mock docker client with the expected output
        let docker_client = Arc::new(FixedResponseDockerClient {
            logs_output: "Test output".to_string(),
            ..Default::default()
        });

        // Create AppContext with mock docker client and test directories
        let ctx = AppContext::builder()
            .with_docker_client(docker_client)
            .build();
        let tsk_config = ctx.tsk_config();

        // Set up a mock .claude.json file in the test claude directory
        let claude_json_path = tsk_config
            .claude_config_dir()
            .join("..")
            .join(".claude.json");
        // Ensure parent directory exists
        if let Some(parent) = claude_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&claude_json_path, "{}").unwrap();

        let task_runner = TaskRunner::new(&ctx);

        // Create a task copy directory
        let repo_hash = crate::storage::get_repo_hash(test_repo.path());
        let task_copy_dir = tsk_config.task_dir("test-task-123", &repo_hash);

        // Use the async filesystem operations to copy the repository
        ctx.file_system()
            .copy_dir(test_repo.path(), &task_copy_dir)
            .await
            .unwrap();

        let task = Task {
            id: "test-task-123".to_string(),
            repo_root: test_repo.path().to_path_buf(),
            name: "test-task".to_string(),
            task_type: "feature".to_string(),
            instructions_file: task_copy_dir
                .join(".tsk/tasks/instructions.md")
                .to_string_lossy()
                .to_string(),
            agent: "claude-code".to_string(),
            timeout: 30,
            status: TaskStatus::Queued,
            created_at: chrono::Local::now(),
            started_at: None,
            completed_at: None,
            branch_name: "tsk/feature/test-task/test-task-123".to_string(),
            error_message: None,
            source_commit: test_repo.get_current_commit().unwrap(),
            tech_stack: "default".to_string(),
            project: "default".to_string(),
            copied_repo_path: task_copy_dir,
            is_interactive: false,
        };

        let result = task_runner.execute_task(&task).await;

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));
    }
}
