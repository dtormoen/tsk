use crate::context::AppContext;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    #[serde(rename = "QUEUED")]
    Queued,
    #[serde(rename = "RUNNING")]
    Running,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "COMPLETE")]
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub repo_root: PathBuf,
    pub name: String,
    pub task_type: String,
    pub description: Option<String>,
    pub instructions_file: Option<String>,
    pub agent: Option<String>,
    pub timeout: u32,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub branch_name: Option<String>,
    pub error_message: Option<String>,
    pub source_commit: Option<String>,
}

impl Task {
    #[allow(dead_code)] // Used in tests
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: PathBuf,
        name: String,
        task_type: String,
        description: Option<String>,
        instructions_file: Option<String>,
        agent: Option<String>,
        timeout: u32,
    ) -> Self {
        let timestamp = Utc::now();
        let id = format!("{}-{}", timestamp.format("%Y-%m-%d-%H%M"), name);

        Self {
            id,
            repo_root,
            name,
            task_type,
            description,
            instructions_file,
            agent,
            timeout,
            status: TaskStatus::Queued,
            created_at: timestamp,
            started_at: None,
            completed_at: None,
            branch_name: None,
            error_message: None,
            source_commit: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_id(
        id: String,
        repo_root: PathBuf,
        name: String,
        task_type: String,
        description: Option<String>,
        instructions_file: Option<String>,
        agent: Option<String>,
        timeout: u32,
    ) -> Self {
        Self {
            id,
            repo_root,
            name,
            task_type,
            description,
            instructions_file,
            agent,
            timeout,
            status: TaskStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            branch_name: None,
            error_message: None,
            source_commit: None,
        }
    }
}

pub struct TaskBuilder {
    repo_root: Option<PathBuf>,
    name: Option<String>,
    task_type: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    edit: bool,
    agent: Option<String>,
    timeout: Option<u32>,
}

impl TaskBuilder {
    pub fn new() -> Self {
        Self {
            repo_root: None,
            name: None,
            task_type: None,
            description: None,
            instructions: None,
            edit: false,
            agent: None,
            timeout: None,
        }
    }

    pub fn repo_root(mut self, repo_root: PathBuf) -> Self {
        self.repo_root = Some(repo_root);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn task_type(mut self, task_type: String) -> Self {
        self.task_type = Some(task_type);
        self
    }

    pub fn description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }

    pub fn instructions(mut self, instructions: Option<String>) -> Self {
        self.instructions = instructions;
        self
    }

    pub fn edit(mut self, edit: bool) -> Self {
        self.edit = edit;
        self
    }

    pub fn agent(mut self, agent: Option<String>) -> Self {
        self.agent = agent;
        self
    }

    pub fn timeout(mut self, timeout: u32) -> Self {
        self.timeout = Some(timeout);
        self
    }

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

        // Validate input
        if self.description.is_none() && self.instructions.is_none() && !self.edit {
            return Err(
                "Either description or instructions must be provided, or use edit mode".into(),
            );
        }

        // Validate agent if specified
        if let Some(ref agent_name) = self.agent {
            if !crate::agent::AgentProvider::is_valid_agent(agent_name) {
                let available_agents = crate::agent::AgentProvider::list_agents().join(", ");
                return Err(format!(
                    "Unknown agent '{}'. Available agents: {}",
                    agent_name, available_agents
                )
                .into());
            }
        }

        // Validate task type
        if task_type != "generic" {
            let template_path = repo_root
                .join("templates")
                .join(format!("{}.md", task_type));
            if !ctx.file_system().exists(&template_path).await? {
                return Err(format!(
                    "No template found for task type '{}'. Please check the templates folder.",
                    task_type
                )
                .into());
            }
        }

        // Create task directory in centralized location
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
        let task_dir_name = format!("{}-{}", timestamp, name);
        let repo_hash = crate::storage::get_repo_hash(&repo_root);
        let task_dir = ctx.xdg_directories().task_dir(&task_dir_name, &repo_hash);
        ctx.file_system().create_dir(&task_dir).await?;

        // Create instructions file
        let instructions_path = if self.edit {
            // Create temporary file in repository root for editing
            let temp_filename = format!(".tsk-edit-{}-instructions.md", task_dir_name);
            let temp_path = repo_root.join(&temp_filename);
            self.write_instructions_content(&temp_path, &task_type, &repo_root, ctx)
                .await?;

            // Open editor with the temporary file
            self.open_editor(temp_path.to_str().ok_or("Invalid path")?)?;
            self.check_instructions_not_empty(&temp_path, ctx).await?;

            // Move the file to the task directory
            let final_path = task_dir.join("instructions.md");
            let content = ctx.file_system().read_file(&temp_path).await?;
            ctx.file_system().write_file(&final_path, &content).await?;
            ctx.file_system().remove_file(&temp_path).await?;

            final_path.to_string_lossy().to_string()
        } else {
            // Create instructions file directly in task directory
            let dest_path = task_dir.join("instructions.md");
            self.write_instructions_content(&dest_path, &task_type, &repo_root, ctx)
                .await?
        };

        // Capture the current commit SHA
        let source_commit = match ctx.git_operations().get_current_commit(&repo_root).await {
            Ok(commit) => Some(commit),
            Err(e) => {
                eprintln!(
                    "Warning: Failed to get current commit for task '{}': {}",
                    name, e
                );
                None
            }
        };

        // Create and return the task
        let mut task = Task::new_with_id(
            task_dir_name.clone(),
            repo_root,
            name,
            task_type,
            None, // description is now stored in instructions file
            Some(instructions_path),
            self.agent,
            timeout,
        );
        task.source_commit = source_commit;

        Ok(task)
    }

    async fn write_instructions_content(
        &self,
        dest_path: &Path,
        task_type: &str,
        repo_root: &Path,
        ctx: &AppContext,
    ) -> Result<String, Box<dyn Error>> {
        let fs = ctx.file_system();

        if let Some(ref inst_path) = self.instructions {
            // Copy existing instructions file
            let content = fs.read_file(Path::new(inst_path)).await?;
            fs.write_file(dest_path, &content).await?;
        } else if let Some(ref desc) = self.description {
            // Check if a template exists for this task type
            let template_path = repo_root
                .join("templates")
                .join(format!("{}.md", task_type));
            let content = if fs.exists(&template_path).await? {
                match fs.read_file(&template_path).await {
                    Ok(template_content) => template_content.replace("{{DESCRIPTION}}", desc),
                    Err(e) => {
                        eprintln!("Warning: Failed to read template file: {}", e);
                        desc.clone()
                    }
                }
            } else {
                desc.clone()
            };

            fs.write_file(dest_path, &content).await?;
        } else {
            // Create empty instructions file for editing
            let template_path = repo_root
                .join("templates")
                .join(format!("{}.md", task_type));
            let initial_content = if fs.exists(&template_path).await? {
                match fs.read_file(&template_path).await {
                    Ok(template_content) => template_content.replace(
                        "{{DESCRIPTION}}",
                        "<!-- TODO: Add your task description here -->",
                    ),
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

    fn open_editor(&self, instructions_path: &str) -> Result<(), Box<dyn Error>> {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
            if std::env::var("VISUAL").is_ok() {
                std::env::var("VISUAL").unwrap()
            } else {
                "vi".to_string()
            }
        });

        println!("Opening instructions file in editor: {}", editor);

        let status = std::process::Command::new(&editor)
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
    use crate::context::file_system::{tests::MockFileSystem, FileSystemOperations};
    use crate::context::AppContext;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_task_builder_basic() {
        let current_dir = std::env::current_dir().unwrap();
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string()),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task = TaskBuilder::new()
            .repo_root(current_dir.clone())
            .name("test-task".to_string())
            .task_type("generic".to_string())
            .description(Some("Test description".to_string()))
            .timeout(60)
            .build(&ctx)
            .await
            .unwrap();

        assert_eq!(task.name, "test-task");
        assert_eq!(task.task_type, "generic");
        assert_eq!(task.timeout, 60);
        assert!(task.instructions_file.is_some());
        assert!(task.id.contains("test-task"));
    }

    #[tokio::test]
    async fn test_task_builder_with_template() {
        let current_dir = std::env::current_dir().unwrap();
        let template_content = "# Feature Template\n\n{{DESCRIPTION}}";
        let template_path = current_dir.join("templates/feature.md");
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string())
                .with_dir(&current_dir.join("templates").to_string_lossy().to_string())
                .with_file(
                    &template_path.to_string_lossy().to_string(),
                    template_content,
                ),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task = TaskBuilder::new()
            .repo_root(current_dir.clone())
            .name("test-feature".to_string())
            .task_type("feature".to_string())
            .description(Some("My feature description".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        assert_eq!(task.task_type, "feature");

        // Verify instructions file was created with template
        let instructions_path = task.instructions_file.as_ref().unwrap();
        let content = fs.read_file(Path::new(instructions_path)).await.unwrap();
        assert_eq!(content, "# Feature Template\n\nMy feature description");
    }

    #[tokio::test]
    async fn test_task_builder_validation_no_input() {
        let current_dir = std::env::current_dir().unwrap();
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string()),
        );

        let ctx = AppContext::builder().with_file_system(fs).build();

        let result = TaskBuilder::new()
            .repo_root(current_dir)
            .name("test-task".to_string())
            .build(&ctx)
            .await;

        assert!(result.is_err());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Either description or instructions")
                || err.contains("Repository root is required")
        );
    }

    #[tokio::test]
    async fn test_task_builder_with_instructions_file() {
        let current_dir = std::env::current_dir().unwrap();
        let instructions_content = "# Instructions for task";
        let instructions_path = current_dir.join("test-instructions.md");
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string())
                .with_file(
                    &instructions_path.to_string_lossy().to_string(),
                    instructions_content,
                ),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task = TaskBuilder::new()
            .repo_root(current_dir.clone())
            .name("test-task".to_string())
            .instructions(Some(instructions_path.to_string_lossy().to_string()))
            .build(&ctx)
            .await
            .unwrap();

        // Verify instructions file was copied
        let task_instructions_path = task.instructions_file.as_ref().unwrap();
        let content = fs
            .read_file(Path::new(task_instructions_path))
            .await
            .unwrap();
        assert_eq!(content, instructions_content);
    }

    #[tokio::test]
    async fn test_task_builder_write_instructions_content() {
        let current_dir = std::env::current_dir().unwrap();
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string()),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task_builder = TaskBuilder::new()
            .name("test-task".to_string())
            .task_type("generic".to_string())
            .description(Some("Test description".to_string()));

        // Test the unified write method
        let temp_path = Path::new(".tsk-edit-2024-01-01-1200-test-task-instructions.md");
        let result_path = task_builder
            .write_instructions_content(temp_path, "generic", &current_dir, &ctx)
            .await
            .unwrap();

        assert_eq!(result_path, temp_path.to_string_lossy().to_string());

        // Verify the content was written
        let content = fs.read_file(temp_path).await.unwrap();
        assert!(content.contains("Test description"));
    }

    #[tokio::test]
    async fn test_task_builder_write_instructions_content_with_template() {
        let current_dir = std::env::current_dir().unwrap();
        let template_content = "# Feature Template\n\n{{DESCRIPTION}}";

        let template_path = current_dir.join("templates/feature.md");
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string())
                .with_dir(&current_dir.join("templates").to_string_lossy().to_string())
                .with_file(
                    &template_path.to_string_lossy().to_string(),
                    template_content,
                ),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task_builder = TaskBuilder::new()
            .name("test-feature".to_string())
            .task_type("feature".to_string())
            .description(Some("My new feature".to_string()));

        let temp_path = Path::new(".tsk-edit-2024-01-01-1200-test-feature-instructions.md");
        task_builder
            .write_instructions_content(temp_path, "feature", &current_dir, &ctx)
            .await
            .unwrap();

        let content = fs.read_file(temp_path).await.unwrap();
        assert!(content.contains("# Feature Template"));
        assert!(content.contains("My new feature"));
        assert!(!content.contains("{{DESCRIPTION}}"));
    }

    #[tokio::test]
    async fn test_task_builder_captures_source_commit() {
        use crate::context::git_operations::tests::MockGitOperations;

        let current_dir = std::env::current_dir().unwrap();
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string()),
        );

        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_get_current_commit_result(Ok(
            "abc123def456789012345678901234567890abcd".to_string()
        ));

        let ctx = AppContext::builder()
            .with_file_system(fs.clone())
            .with_git_operations(mock_git_ops.clone())
            .build();

        let task = TaskBuilder::new()
            .repo_root(current_dir.clone())
            .name("test-task".to_string())
            .task_type("generic".to_string())
            .description(Some("Test description".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        // Verify the source commit was captured
        assert_eq!(
            task.source_commit,
            Some("abc123def456789012345678901234567890abcd".to_string())
        );

        // Verify the git operation was called
        let calls = mock_git_ops.get_get_current_commit_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], current_dir.to_string_lossy().to_string());
    }

    #[tokio::test]
    async fn test_task_builder_handles_source_commit_error() {
        use crate::context::git_operations::tests::MockGitOperations;

        let current_dir = std::env::current_dir().unwrap();
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string()),
        );

        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_get_current_commit_result(Err("Not a git repository".to_string()));

        let ctx = AppContext::builder()
            .with_file_system(fs.clone())
            .with_git_operations(mock_git_ops.clone())
            .build();

        let task = TaskBuilder::new()
            .repo_root(current_dir.clone())
            .name("test-task".to_string())
            .task_type("generic".to_string())
            .description(Some("Test description".to_string()))
            .build(&ctx)
            .await
            .unwrap();

        // Verify the task was created successfully even though getting commit failed
        assert_eq!(task.name, "test-task");
        assert_eq!(task.source_commit, None);

        // Verify the git operation was called
        let calls = mock_git_ops.get_get_current_commit_calls();
        assert_eq!(calls.len(), 1);
    }
}
