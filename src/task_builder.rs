use crate::assets::{AssetManager, layered::LayeredAssetManager};
use crate::context::AppContext;
use crate::context::tsk_config::TskConfig;
use crate::git::RepoManager;
use crate::task::Task;
use crate::utils::sanitize_for_branch_name;
use chrono::Local;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Builder for creating tasks with a fluent API
pub struct TaskBuilder {
    repo_root: Option<PathBuf>,
    name: Option<String>,
    task_type: Option<String>,
    description: Option<String>,
    /// Path to a file containing instructions
    instructions_file_path: Option<PathBuf>,
    edit: bool,
    agent: Option<String>,
    timeout: Option<u32>,
    tech_stack: Option<String>,
    project: Option<String>,
    copied_repo_path: Option<PathBuf>,
    is_interactive: bool,
}

impl TaskBuilder {
    /// Creates a new TaskBuilder instance
    pub fn new() -> Self {
        Self {
            repo_root: None,
            name: None,
            task_type: None,
            description: None,
            instructions_file_path: None,
            edit: false,
            agent: None,
            timeout: None,
            tech_stack: None,
            project: None,
            copied_repo_path: None,
            is_interactive: false,
        }
    }

    /// Creates a TaskBuilder from an existing task. This is used for retrying tasks.
    pub fn from_existing(task: &Task) -> Self {
        let mut builder = Self::new();
        builder.repo_root = Some(task.repo_root.clone());
        builder.name = Some(task.name.clone());
        builder.task_type = Some(task.task_type.clone());
        builder.agent = Some(task.agent.clone());
        builder.timeout = Some(task.timeout);
        builder.tech_stack = Some(task.tech_stack.clone());
        builder.project = Some(task.project.clone());
        builder.copied_repo_path = Some(task.copied_repo_path.clone());
        builder.is_interactive = task.is_interactive;

        // Copy the instructions file path
        builder.instructions_file_path = Some(PathBuf::from(&task.instructions_file));

        builder
    }

    /// Sets the repository root path
    pub fn repo_root(mut self, repo_root: PathBuf) -> Self {
        self.repo_root = Some(repo_root);
        self
    }

    /// Sets the task name
    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    /// Sets the task type
    pub fn task_type(mut self, task_type: String) -> Self {
        self.task_type = Some(task_type);
        self
    }

    /// Sets the task description
    pub fn description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }

    /// Sets the path to a file containing instructions
    pub fn instructions_file(mut self, path: Option<PathBuf>) -> Self {
        self.instructions_file_path = path;
        self
    }

    /// Sets whether to open an editor for instructions
    pub fn edit(mut self, edit: bool) -> Self {
        self.edit = edit;
        self
    }

    /// Sets the AI agent to use
    pub fn agent(mut self, agent: Option<String>) -> Self {
        self.agent = agent;
        self
    }

    /// Sets the task timeout in minutes
    pub fn timeout(mut self, timeout: u32) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the technology stack for Docker image selection
    pub fn tech_stack(mut self, tech_stack: Option<String>) -> Self {
        self.tech_stack = tech_stack;
        self
    }

    /// Sets the project name for Docker image selection
    pub fn project(mut self, project: Option<String>) -> Self {
        self.project = project;
        self
    }

    /// Sets whether the task should run in interactive mode
    #[allow(dead_code)]
    pub fn with_interactive(mut self, is_interactive: bool) -> Self {
        self.is_interactive = is_interactive;
        self
    }

    /// Builds the task, creating all necessary files and directories
    pub async fn build(self, ctx: &AppContext) -> Result<Task, Box<dyn Error>> {
        let repo_root = self
            .repo_root
            .clone()
            .ok_or("Repository root is required")?;
        let name = self.name.clone().ok_or("Task name is required")?;
        let task_type = self
            .task_type
            .clone()
            .unwrap_or_else(|| "generic".to_string());
        let timeout = self.timeout.unwrap_or(30);

        // Check if template requires description
        let template_needs_description = if task_type != "generic" {
            // Create asset manager for template validation
            let asset_manager =
                LayeredAssetManager::new_with_standard_layers(Some(&repo_root), &ctx.tsk_config());
            match asset_manager.get_template(&task_type) {
                Ok(template_content) => template_content.contains("{{DESCRIPTION}}"),
                Err(_) => true, // If we can't read the template, assume it needs description
            }
        } else {
            true // Generic tasks always need description
        };

        // Validate input
        if template_needs_description
            && self.description.is_none()
            && self.instructions_file_path.is_none()
            && !self.edit
        {
            return Err(
                "Either description or prompt file must be provided, or use edit mode".into(),
            );
        }

        // Get agent or use default
        let agent = self
            .agent
            .clone()
            .unwrap_or_else(|| crate::agent::AgentProvider::default_agent().to_string());

        // Validate agent
        if !crate::agent::AgentProvider::is_valid_agent(&agent) {
            let available_agents = crate::agent::AgentProvider::list_agents().join(", ");
            return Err(
                format!("Unknown agent '{agent}'. Available agents: {available_agents}").into(),
            );
        }

        // Validate task type
        if task_type != "generic" {
            // Create asset manager for template validation
            let asset_manager =
                LayeredAssetManager::new_with_standard_layers(Some(&repo_root), &ctx.tsk_config());
            let available_templates = asset_manager.list_templates();
            if !available_templates.contains(&task_type.to_string()) {
                return Err(format!(
                    "No template found for task type '{}'. Available templates: {}",
                    task_type,
                    available_templates.join(", ")
                )
                .into());
            }
        }

        // Create task directory in centralized location
        let now = Local::now();
        let created_at = now;
        let id = nanoid::nanoid!(8);
        let task_dir_name = id.clone();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_dir = ctx.tsk_config().task_dir(&task_dir_name, &repo_hash);
        ctx.file_system().create_dir(&task_dir).await?;

        // Create instructions file
        let instructions_path = if self.edit {
            // Create temporary file in repository root for editing
            let temp_filename = format!(".tsk-edit-{task_dir_name}-instructions.md");
            let temp_path = repo_root.join(&temp_filename);
            self.write_instructions_content(&temp_path, &task_type, ctx)
                .await?;

            // Open editor with the temporary file
            let tsk_config = ctx.tsk_config();
            self.open_editor(temp_path.to_str().ok_or("Invalid path")?, &tsk_config)?;

            // Check if file is empty and ensure cleanup happens even on error
            let needs_cleanup = self
                .check_instructions_not_empty(&temp_path, ctx)
                .await
                .is_err();

            if needs_cleanup {
                // Clean up the temporary file and task directory before returning the error
                let _ = ctx.file_system().remove_file(&temp_path).await;
                let _ = ctx.file_system().remove_dir(&task_dir).await;
                return Err("Instructions file is empty. Task creation cancelled.".into());
            }

            // Move the file to the task directory
            let final_path = task_dir.join("instructions.md");
            let content = ctx.file_system().read_file(&temp_path).await?;
            ctx.file_system().write_file(&final_path, &content).await?;
            ctx.file_system().remove_file(&temp_path).await?;

            final_path.to_string_lossy().to_string()
        } else {
            // Create instructions file directly in task directory
            let dest_path = task_dir.join("instructions.md");
            self.write_instructions_content(&dest_path, &task_type, ctx)
                .await?
        };

        // Capture the current commit SHA
        let source_commit = match ctx.git_operations().get_current_commit(&repo_root).await {
            Ok(commit) => commit,
            Err(e) => {
                return Err(format!("Failed to get current commit for task '{name}': {e}").into());
            }
        };

        // Auto-detect tech_stack and project if not provided
        let tech_stack = match self.tech_stack {
            Some(ts) => {
                println!("Using tech stack: {ts}");
                ts
            }
            None => match crate::repository::detect_tech_stack(&repo_root).await {
                Ok(detected) => {
                    println!("Auto-detected tech stack: {detected}");
                    detected
                }
                Err(e) => {
                    eprintln!("Warning: Failed to detect tech stack: {e}. Using default.");
                    "default".to_string()
                }
            },
        };

        let project = match self.project {
            Some(p) => {
                println!("Using project: {p}");
                p
            }
            None => match crate::repository::detect_project_name(&repo_root).await {
                Ok(detected) => {
                    println!("Auto-detected project name: {detected}");
                    detected
                }
                Err(e) => {
                    eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                    "default".to_string()
                }
            },
        };

        // Generate human-readable branch name with format: tsk/{task-type}/{task-name}/{task-id}
        let sanitized_task_type = sanitize_for_branch_name(&task_type);
        let sanitized_name = sanitize_for_branch_name(&name);
        let branch_name = format!("tsk/{sanitized_task_type}/{sanitized_name}/{id}");

        // Copy the repository for the task
        let repo_manager = RepoManager::new(
            ctx.tsk_config(),
            ctx.file_system(),
            ctx.git_operations(),
            ctx.git_sync_manager(),
        );

        let (copied_repo_path, _) = repo_manager
            .copy_repo(
                &task_dir_name,
                &repo_root,
                Some(&source_commit),
                &branch_name,
            )
            .await
            .map_err(|e| format!("Failed to copy repository: {e}"))?;

        // Create and return the task
        let task = Task::new(
            id,
            repo_root,
            name,
            task_type,
            instructions_path,
            agent,
            timeout,
            branch_name,
            source_commit,
            tech_stack,
            project,
            created_at,
            copied_repo_path,
            self.is_interactive,
        );

        Ok(task)
    }

    async fn write_instructions_content(
        &self,
        dest_path: &Path,
        task_type: &str,
        ctx: &AppContext,
    ) -> Result<String, Box<dyn Error>> {
        let fs = ctx.file_system();

        if let Some(ref file_path) = self.instructions_file_path {
            // File path provided - read and copy content
            let content = fs.read_file(file_path).await?;
            fs.write_file(dest_path, &content).await?;
        } else if let Some(ref desc) = self.description {
            // Check if a template exists for this task type
            let content = if task_type != "generic" {
                // Create asset manager for template retrieval
                let asset_manager = LayeredAssetManager::new_with_standard_layers(
                    self.repo_root.as_deref(),
                    &ctx.tsk_config(),
                );
                match asset_manager.get_template(task_type) {
                    Ok(template_content) => template_content.replace("{{DESCRIPTION}}", desc),
                    Err(e) => {
                        eprintln!("Warning: Failed to read template: {e}");
                        desc.clone()
                    }
                }
            } else {
                desc.clone()
            };

            fs.write_file(dest_path, &content).await?;
        } else {
            // No description provided - use template as-is or create empty file
            let initial_content = if task_type != "generic" {
                // Create asset manager for template retrieval
                let asset_manager = LayeredAssetManager::new_with_standard_layers(
                    self.repo_root.as_deref(),
                    &ctx.tsk_config(),
                );
                match asset_manager.get_template(task_type) {
                    Ok(template_content) => {
                        // If template has description placeholder and we're in edit mode, add TODO
                        if self.edit && template_content.contains("{{DESCRIPTION}}") {
                            template_content.replace(
                                "{{DESCRIPTION}}",
                                "<!-- TODO: Add your task description here -->",
                            )
                        } else {
                            // Use template as-is (for templates without description placeholder)
                            template_content
                        }
                    }
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            };

            fs.write_file(dest_path, &initial_content).await?;
        }

        println!("Created instructions file: {}", dest_path.display());
        Ok(dest_path.to_string_lossy().to_string())
    }

    fn open_editor(
        &self,
        instructions_path: &str,
        tsk_config: &TskConfig,
    ) -> Result<(), Box<dyn Error>> {
        let editor = tsk_config.editor();

        println!("Opening instructions file in editor: {}", editor);

        let status = std::process::Command::new(editor)
            .arg(instructions_path)
            .status()?;

        if !status.success() {
            return Err("Editor exited with non-zero status".into());
        }

        Ok(())
    }

    async fn check_instructions_not_empty(
        &self,
        instructions_path: &Path,
        ctx: &AppContext,
    ) -> Result<(), Box<dyn Error>> {
        // Check if file is empty after editing
        let content = ctx.file_system().read_file(instructions_path).await?;
        if content.trim().is_empty() {
            return Err("Instructions file is empty. Task creation cancelled.".into());
        }
        Ok(())
    }
}

impl Default for TaskBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use tempfile::TempDir;

    /// Helper to create a standard test context with real file system and git operations
    /// Returns (git_repo, AppContext)
    async fn create_test_context() -> (crate::test_utils::TestGitRepository, AppContext) {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let ctx = AppContext::builder().build();

        (test_repo, ctx)
    }

    /// Helper function to create a basic task for testing
    async fn create_basic_task(name: &str, description: &str) -> Task {
        let (test_repo, ctx) = create_test_context().await;
        let current_dir = test_repo.path().to_path_buf();

        TaskBuilder::new()
            .repo_root(current_dir)
            .name(name.to_string())
            .task_type("generic".to_string())
            .description(Some(description.to_string()))
            .build(&ctx)
            .await
            .unwrap()
    }

    /// Helper to verify task instructions content
    async fn verify_instructions_content(ctx: &AppContext, task: &Task, expected_content: &str) {
        let instructions_path = ctx
            .tsk_config()
            .data_dir()
            .join("tasks")
            .join(&task.id)
            .join(&task.instructions_file);
        let content = ctx
            .file_system()
            .read_file(&instructions_path)
            .await
            .unwrap();
        assert!(content.contains(expected_content));
    }

    #[tokio::test]
    async fn test_task_builder_basic() {
        let task = create_basic_task("test-task", "Test description").await;

        assert_eq!(task.name, "test-task");
        assert_eq!(task.task_type, "generic");
        assert!(!task.instructions_file.is_empty());
        assert_eq!(task.id.len(), 8);
        assert_eq!(task.timeout, 30); // default timeout
    }

    #[tokio::test]
    async fn test_task_builder_with_custom_properties() {
        let (test_repo, ctx) = create_test_context().await;
        let current_dir = test_repo.path().to_path_buf();

        let task = TaskBuilder::new()
            .repo_root(current_dir)
            .name("custom-task".to_string())
            .task_type("feat".to_string())
            .description(Some("Custom description".to_string()))
            .timeout(60)
            .tech_stack(Some("rust".to_string()))
            .project(Some("web-api".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        assert_eq!(task.name, "custom-task");
        assert_eq!(task.task_type, "feat");
        assert_eq!(task.timeout, 60);
        assert_eq!(task.tech_stack, "rust");
        assert_eq!(task.project, "web-api");
    }

    #[tokio::test]
    async fn test_task_builder_with_template() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let current_dir = test_repo.path().to_path_buf();

        // Create template file
        let template_content = "# Feature Template\n\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/feat.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let task = TaskBuilder::new()
            .repo_root(current_dir)
            .name("test-feature".to_string())
            .task_type("feat".to_string())
            .description(Some("My new feature".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        assert_eq!(task.task_type, "feat");
        verify_instructions_content(&ctx, &task, "Feature Template").await;
        verify_instructions_content(&ctx, &task, "My new feature").await;
    }

    #[tokio::test]
    async fn test_task_builder_validation_errors() {
        let temp_dir = TempDir::new().unwrap();
        let ctx = AppContext::builder().build();

        // Test missing repository root
        let result = TaskBuilder::new()
            .name("test".to_string())
            .build(&ctx)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Repository root"));

        // Test missing task name
        let result = TaskBuilder::new()
            .repo_root(temp_dir.path().to_path_buf())
            .build(&ctx)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Task name"));

        // Test missing description for generic task
        let result = TaskBuilder::new()
            .repo_root(temp_dir.path().to_path_buf())
            .name("test".to_string())
            .build(&ctx)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Either description or prompt file")
        );
    }

    #[tokio::test]
    async fn test_task_builder_with_instructions_file() {
        let (test_repo, ctx) = create_test_context().await;
        let current_dir = test_repo.path().to_path_buf();

        // Create instructions file
        let instructions_content = "# Instructions for task\n\nDetailed steps here.";
        let instructions_path = current_dir.join("task-instructions.md");
        ctx.file_system()
            .write_file(&instructions_path, instructions_content)
            .await
            .unwrap();

        let task = TaskBuilder::new()
            .repo_root(current_dir)
            .name("file-task".to_string())
            .instructions_file(Some(instructions_path))
            .build(&ctx)
            .await
            .unwrap();

        verify_instructions_content(&ctx, &task, "Instructions for task").await;
        verify_instructions_content(&ctx, &task, "Detailed steps here").await;
    }

    #[tokio::test]
    async fn test_task_builder_branch_name_generation() {
        let task = create_basic_task("my-feature-name", "Description").await;

        // Branch name should follow pattern: tsk/{task-type}/{task-name}/{task-id}
        assert!(task.branch_name.starts_with("tsk/generic/my-feature-name/"));
        assert_eq!(task.branch_name.split('/').count(), 4);
    }

    #[tokio::test]
    async fn test_task_builder_captures_source_commit() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        let initial_commit = test_repo.init_with_commit().unwrap();

        // Add another commit
        test_repo.create_file("new_file.txt", "content").unwrap();
        test_repo.stage_all().unwrap();
        let current_commit = test_repo.commit("Add new file").unwrap();

        let ctx = AppContext::builder().build();

        let task = TaskBuilder::new()
            .repo_root(test_repo.path().to_path_buf())
            .name("test-task".to_string())
            .description(Some("Test".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        assert_eq!(task.source_commit, current_commit);
        assert_ne!(task.source_commit, initial_commit);
    }

    #[tokio::test]
    async fn test_task_builder_interactive_mode() {
        let (test_repo, ctx) = create_test_context().await;

        let task = TaskBuilder::new()
            .repo_root(test_repo.path().to_path_buf())
            .name("interactive-task".to_string())
            .description(Some("Test".to_string()))
            .with_interactive(true)
            .build(&ctx)
            .await
            .unwrap();

        assert!(task.is_interactive);

        // Default should be non-interactive
        let task2 = TaskBuilder::new()
            .repo_root(test_repo.path().to_path_buf())
            .name("regular-task".to_string())
            .description(Some("Test".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        assert!(!task2.is_interactive);
    }

    #[tokio::test]
    async fn test_task_builder_repository_copy() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Add some files to verify they get copied
        test_repo
            .create_file("src/main.rs", "fn main() {}")
            .unwrap();
        test_repo
            .create_file("Cargo.toml", "[package]\nname = \"test\"")
            .unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add source files").unwrap();

        let ctx = AppContext::builder().build();

        let task = TaskBuilder::new()
            .repo_root(test_repo.path().to_path_buf())
            .name("copy-test".to_string())
            .description(Some("Test repository copy".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        // Verify repository was copied to the correct location
        let repo_hash = crate::storage::get_repo_hash(test_repo.path());
        let task_dir = ctx.tsk_config().task_dir(&task.id, &repo_hash);
        let copied_repo = task_dir.join("repo");

        assert!(copied_repo.exists());
        assert!(copied_repo.join("src/main.rs").exists());
        assert!(copied_repo.join("Cargo.toml").exists());
        assert!(copied_repo.join(".git").exists());
    }
}
