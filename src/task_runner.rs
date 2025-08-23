use crate::agent::AgentProvider;
use crate::assets::layered::LayeredAssetManager;
use crate::docker::{
    DockerManager, composer::DockerComposer, image_manager::DockerImageManager,
    template_manager::DockerTemplateManager,
};
use crate::git::RepoManager;
use crate::notifications::NotificationClient;
use crate::task::Task;
use std::path::PathBuf;
use std::sync::Arc;

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

pub struct TaskRunner {
    repo_manager: RepoManager,
    docker_manager: DockerManager,
    notification_client: Arc<dyn NotificationClient>,
    docker_client: Arc<dyn crate::context::docker_client::DockerClient>,
    xdg_directories: Arc<crate::storage::XdgDirectories>,
}

impl TaskRunner {
    pub fn new(
        repo_manager: RepoManager,
        docker_manager: DockerManager,
        _docker_image_manager: Arc<DockerImageManager>,
        notification_client: Arc<dyn NotificationClient>,
        docker_client: Arc<dyn crate::context::docker_client::DockerClient>,
        xdg_directories: Arc<crate::storage::XdgDirectories>,
    ) -> Self {
        Self {
            repo_manager,
            docker_manager,
            notification_client,
            docker_client,
            xdg_directories,
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
            let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
                Some(&repo_path),
                &self.xdg_directories,
            ));
            let template_manager =
                DockerTemplateManager::new(asset_manager.clone(), self.xdg_directories.clone());
            let composer = DockerComposer::new(DockerTemplateManager::new(
                asset_manager,
                self.xdg_directories.clone(),
            ));
            let task_image_manager =
                DockerImageManager::new(self.docker_client.clone(), template_manager, composer);

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
                    is_interactive,
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
    use crate::context::file_system::{DefaultFileSystem, FileSystemOperations};
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::{FixedResponseDockerClient, git_test_utils::TestGitRepository};
    use std::sync::Arc;

    #[tokio::test]
    #[ignore = "Test requires Docker to be available for image building"]
    async fn test_execute_task_success() {
        use crate::git::RepoManager;

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

        // Set up a temporary home directory with a mock .claude.json file
        let temp_home = tempfile::tempdir().unwrap();
        let claude_json_path = temp_home.path().join(".claude.json");
        std::fs::write(&claude_json_path, "{}").unwrap();

        // Note: Currently agents are created through AgentProvider which doesn't pass Config.
        // Until AgentProvider is refactored to accept Config, we still need to set HOME env var.
        let original_home = std::env::var("HOME").ok();
        unsafe {
            std::env::set_var("HOME", temp_home.path());
        }

        // Use DefaultFileSystem for real file operations
        let fs = Arc::new(DefaultFileSystem);
        let git_ops = Arc::new(crate::context::git_operations::DefaultGitOperations);
        let docker_client = Arc::new(FixedResponseDockerClient::default());

        // Create test XDG directories
        let temp_xdg = tempfile::tempdir().unwrap();
        let xdg_config = crate::storage::XdgConfig::with_paths(
            temp_xdg.path().join("xdg-data"),
            temp_xdg.path().join("xdg-runtime"),
            temp_xdg.path().join("xdg-config"),
        );
        let xdg_directories =
            Arc::new(crate::storage::XdgDirectories::new(Some(xdg_config)).unwrap());
        xdg_directories.ensure_directories().unwrap();

        let git_sync = Arc::new(crate::git_sync::GitSyncManager::new());
        let repo_manager = RepoManager::new(xdg_directories.clone(), fs.clone(), git_ops, git_sync);
        let docker_manager = crate::docker::DockerManager::new(docker_client.clone());

        // Create docker image manager
        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::docker::composer::DockerComposer;
        use crate::docker::template_manager::DockerTemplateManager;

        let template_manager =
            DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), xdg_directories.clone());
        let composer = DockerComposer::new(DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            xdg_directories.clone(),
        ));
        let docker_image_manager = Arc::new(DockerImageManager::new(
            docker_client.clone(),
            template_manager,
            composer,
        ));

        let notification_client = Arc::new(crate::notifications::NoOpNotificationClient);
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            docker_image_manager,
            notification_client,
            docker_client,
            xdg_directories.clone(),
        );

        // Create a task copy directory
        let repo_hash = crate::storage::get_repo_hash(test_repo.path());
        let task_copy_dir = xdg_directories.task_dir("test-task-123", &repo_hash);

        // Use the async filesystem operations to copy the repository
        fs.copy_dir(test_repo.path(), &task_copy_dir).await.unwrap();

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
        };

        let result = task_runner.execute_task(&task, false).await;

        // Restore original HOME
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
        }

        assert!(result.is_ok(), "Error: {:?}", result.as_ref().err());
        let execution_result = result.unwrap();
        assert_eq!(execution_result.output, "Test output");
        assert!(execution_result.branch_name.contains("test-task"));
    }
}
