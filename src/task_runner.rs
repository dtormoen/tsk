use crate::agent::AgentProvider;
use crate::context::AppContext;
use crate::docker::{DockerManager, image_manager::DockerImageManager};
use crate::git::RepoManager;
use crate::task::{Task, TaskStatus};
use crate::task_storage::TaskStorage;
use std::sync::Arc;

/// Result of executing a task
///
/// Contains the branch name where the task's changes were committed
/// and the success message from the agent.
#[derive(Debug)]
pub struct TaskExecutionResult {
    pub branch_name: String,
    pub message: String,
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
/// - Storing tasks and updating status throughout execution
/// - Agent validation and warmup
/// - Docker image management and container execution
/// - Repository changes and git operations
/// - Task completion notifications
pub struct TaskRunner {
    task_storage: Arc<dyn TaskStorage>,
    ctx: AppContext,
    repo_manager: RepoManager,
    docker_manager: DockerManager,
}

impl TaskRunner {
    /// Creates a new TaskRunner with the given DockerManager.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context providing all required dependencies
    /// * `docker_manager` - The DockerManager for container operations
    pub fn new(ctx: &AppContext, docker_manager: DockerManager) -> Self {
        Self {
            task_storage: ctx.task_storage(),
            ctx: ctx.clone(),
            repo_manager: RepoManager::new(ctx),
            docker_manager,
        }
    }

    /// Stores and executes a task inline, for use by `run` and `shell` commands.
    ///
    /// Unlike server-scheduled tasks (which are added to the queue as `Queued` and later picked
    /// up by the scheduler), `run` and `shell` execute tasks directly. This method persists the
    /// task as `Running` before execution so it appears in `tsk list` and can be used as a
    /// parent for task chaining. The `Running` status prevents the server scheduler from
    /// picking it up.
    pub async fn store_and_run(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        let mut stored_task = task.clone();
        stored_task.status = TaskStatus::Running;
        stored_task.started_at = Some(chrono::Utc::now());
        if let Err(e) = self.task_storage.add_task(stored_task).await {
            eprintln!("Error storing task: {e}");
        }
        self.run_with_lifecycle(task).await
    }

    /// Execute a task from the queue with status updates.
    ///
    /// Transitions the task to Running, then delegates to `run_with_lifecycle` which
    /// handles execution and completion status updates.
    pub async fn run_queued(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        if let Err(e) = self.task_storage.mark_running(&task.id).await {
            eprintln!("Error updating task status: {e}");
        }
        self.run_with_lifecycle(task).await
    }

    /// Execute a task in a Docker container and update storage with the result.
    ///
    /// Callers are responsible for getting the task into storage and setting
    /// the initial Running state before calling this method. This method handles:
    /// - Agent validation and warmup
    /// - Docker image management and container execution
    /// - Repository changes and git operations
    /// - Task completion notifications
    /// - Updating task storage to Complete or Failed
    async fn run_with_lifecycle(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        let result = self.run_in_container(task).await;
        match &result {
            Ok(exec_result) => {
                self.ctx.notification_client().notify_task_complete(
                    &task.name,
                    true,
                    Some(&exec_result.message),
                );
                if let Err(e) = self
                    .task_storage
                    .mark_complete(&task.id, &exec_result.branch_name)
                    .await
                {
                    eprintln!("Error updating task status: {e}");
                }
            }
            Err(e) => {
                self.ctx.notification_client().notify_task_complete(
                    &task.name,
                    false,
                    Some(&e.message),
                );
                if let Err(storage_err) =
                    self.task_storage.mark_failed(&task.id, &e.message).await
                {
                    eprintln!("Error updating task status: {storage_err}");
                }
            }
        }
        result
    }

    /// Run a task in a Docker container.
    ///
    /// Contains the core execution logic: agent setup, Docker image building,
    /// container execution, and git operations. Does not handle storage updates
    /// or notifications.
    async fn run_in_container(
        &self,
        task: &Task,
    ) -> Result<TaskExecutionResult, TaskExecutionError> {
        // Get the agent for this task
        let agent = AgentProvider::get_agent(&task.agent, self.ctx.tsk_env())
            .map_err(|e| format!("Error getting agent: {e}"))?;

        // Validate the agent
        agent
            .validate()
            .await
            .map_err(|e| format!("Agent validation failed: {e}"))?;

        // Use the pre-copied repository path
        // Child tasks will have this set by the scheduler before execution
        let repo_path = task.copied_repo_path.as_ref().ok_or_else(|| {
            format!(
                "Task '{}' has no copied repository. This may indicate the task is waiting for its parent to complete.",
                task.id
            )
        })?;
        let branch_name = task.branch_name.clone();

        println!("Using repository copy at: {}", repo_path.display());

        // Launch Docker container
        println!("Launching Docker container with {} agent...", agent.name());
        println!("\n{}", "=".repeat(60));

        // Create a task-specific image manager with the copied repository as the project root
        // This ensures that project-specific dockerfiles are found in the copied repository
        let task_image_manager = DockerImageManager::new(
            &self.ctx,
            self.docker_manager.client(),
            Some(repo_path.as_path()),
            None,
        );

        // Ensure the Docker image exists - always rebuild to pick up any changes
        let docker_image_tag = task_image_manager
            .ensure_image(
                &task.stack,
                &task.agent,
                Some(&task.project),
                Some(repo_path.as_path()),
                true,
            )
            .await
            .map_err(|e| format!("Error ensuring Docker image: {e}"))?;

        // Run agent warmup
        if let Err(e) = agent.warmup().await {
            return Err(TaskExecutionError {
                message: format!("Agent warmup failed: {e}"),
                is_warmup_failure: true,
            });
        }

        // Run the container using the unified method
        let (_output, task_result) = match self
            .docker_manager
            .run_task_container(&docker_image_tag, task, agent.as_ref())
            .await
        {
            Ok(result) => result,
            Err(e) => {
                return Err(format!("Error running container: {e}").into());
            }
        };

        println!("\n{}", "=".repeat(60));
        if task_result.success {
            println!("Container execution completed successfully");
        } else {
            println!(
                "Container execution completed with failure: {}",
                task_result.message
            );
        }

        // Commit any changes made by the container
        let commit_message = format!("TSK automated changes for task: {}", task.name);
        if let Err(e) = self
            .repo_manager
            .commit_changes(repo_path, &commit_message)
            .await
        {
            eprintln!("Error committing changes: {e}");
        }

        // Fetch changes back to main repository
        match self
            .repo_manager
            .fetch_changes(
                repo_path,
                &branch_name,
                &task.repo_root,
                &task.source_commit,
                task.source_branch.as_deref(),
                self.ctx.tsk_config().git_town.enabled,
            )
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

        if task_result.success {
            Ok(TaskExecutionResult {
                branch_name,
                message: task_result.message,
            })
        } else {
            Err(TaskExecutionError {
                message: task_result.message,
                is_warmup_failure: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::git_test_utils::TestGitRepository;

    #[tokio::test]
    async fn test_store_and_run_success() {
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

        // Create AppContext with test directories
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        // Set up a mock .claude.json file in the test claude directory
        let claude_json_path = tsk_env.claude_config_dir().join("..").join(".claude.json");
        // Ensure parent directory exists
        if let Some(parent) = claude_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&claude_json_path, "{}").unwrap();

        let docker_manager = DockerManager::new(&ctx, docker_client);
        let task_runner = TaskRunner::new(&ctx, docker_manager);

        // Create a task copy directory
        let task_copy_dir = tsk_env.task_dir("test-task-123");

        // Use the async filesystem operations to copy the repository
        crate::file_system::copy_dir(test_repo.path(), &task_copy_dir)
            .await
            .unwrap();

        let task = Task {
            id: "test-task-123".to_string(),
            repo_root: test_repo.path().to_path_buf(),
            task_type: "feature".to_string(),
            instructions_file: task_copy_dir
                .join(".tsk/tasks/instructions.md")
                .to_string_lossy()
                .to_string(),
            branch_name: "tsk/feature/test-task/test-task-123".to_string(),
            source_commit: test_repo.get_current_commit().unwrap(),
            copied_repo_path: Some(task_copy_dir),
            ..Task::test_default()
        };

        let result = task_runner.store_and_run(&task).await;

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert!(execution_result.branch_name.contains("test-task"));

        // Verify task was stored and marked complete
        let stored = ctx.task_storage().get_task("test-task-123").await.unwrap();
        assert_eq!(stored.unwrap().status, TaskStatus::Complete);
    }

    #[tokio::test]
    async fn test_store_and_run_infrastructure_failure() {
        use crate::context::AppContext;
        use crate::test_utils::FixedResponseDockerClient;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        test_repo
            .create_file(".tsk/tasks/instructions.md", "Test task instructions")
            .unwrap();
        test_repo.create_file("test.txt", "test content").unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add test files").unwrap();

        // Docker client that fails on container start (infrastructure failure)
        let docker_client = Arc::new(FixedResponseDockerClient {
            should_fail_start: true,
            ..Default::default()
        });

        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let claude_json_path = tsk_env.claude_config_dir().join("..").join(".claude.json");
        if let Some(parent) = claude_json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&claude_json_path, "{}").unwrap();

        let docker_manager = DockerManager::new(&ctx, docker_client);
        let task_runner = TaskRunner::new(&ctx, docker_manager);
        let task_copy_dir = tsk_env.task_dir("infra-fail-123");

        crate::file_system::copy_dir(test_repo.path(), &task_copy_dir)
            .await
            .unwrap();

        let task = Task {
            id: "infra-fail-123".to_string(),
            repo_root: test_repo.path().to_path_buf(),
            task_type: "feature".to_string(),
            instructions_file: task_copy_dir
                .join(".tsk/tasks/instructions.md")
                .to_string_lossy()
                .to_string(),
            branch_name: "tsk/feature/infra-fail/infra-fail-123".to_string(),
            source_commit: test_repo.get_current_commit().unwrap(),
            copied_repo_path: Some(task_copy_dir),
            ..Task::test_default()
        };

        let result = task_runner.store_and_run(&task).await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.message.contains("Error running container"),
            "Expected infrastructure error, got: {}",
            error.message
        );
        assert!(!error.is_warmup_failure);

        // Verify task was stored and marked failed
        let stored = ctx
            .task_storage()
            .get_task("infra-fail-123")
            .await
            .unwrap();
        assert_eq!(stored.unwrap().status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_store_and_run() {
        use crate::context::AppContext;
        use crate::context::docker_client::DockerClient;
        use crate::test_utils::NoOpDockerClient;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let task_id = "store-exec-1".to_string();
        let task_dir_path = tsk_env.task_dir(&task_id);
        std::fs::create_dir_all(&task_dir_path).unwrap();
        let instructions_path = task_dir_path.join("instructions.md");
        std::fs::write(&instructions_path, "Test instructions").unwrap();

        let task = Task {
            id: task_id.clone(),
            repo_root: test_repo.path().to_path_buf(),
            name: "store-exec-task".to_string(),
            branch_name: format!("tsk/{task_id}"),
            copied_repo_path: Some(task_dir_path.join("repo")),
            ..Task::test_default()
        };

        let docker_client: Arc<dyn DockerClient> = Arc::new(NoOpDockerClient);
        let docker_manager = DockerManager::new(&ctx, docker_client);
        let task_runner = TaskRunner::new(&ctx, docker_manager);
        let _result = task_runner.store_and_run(&task).await;

        // Verify the task exists in storage after execution
        let storage = ctx.task_storage();
        let stored = storage.get_task(&task_id).await.unwrap();
        assert!(stored.is_some(), "Task should exist in storage");

        let stored_task = stored.unwrap();
        assert_ne!(
            stored_task.status,
            TaskStatus::Queued,
            "Task should not be Queued after store_and_run"
        );
        assert!(
            stored_task.started_at.is_some(),
            "Task should have started_at set"
        );

        // Verify the task appears in list_tasks
        let all_tasks = storage.list_tasks().await.unwrap();
        assert!(
            all_tasks.iter().any(|t| t.id == task_id),
            "Task should appear in the list of all tasks"
        );
    }
}
