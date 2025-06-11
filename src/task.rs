use crate::context::AppContext;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::Path;

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
}

impl Task {
    #[allow(dead_code)] // Used in tests
    pub fn new(
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
        }
    }

    pub fn new_with_id(
        id: String,
        name: String,
        task_type: String,
        description: Option<String>,
        instructions_file: Option<String>,
        agent: Option<String>,
        timeout: u32,
    ) -> Self {
        Self {
            id,
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
        }
    }
}

pub struct TaskBuilder {
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
            name: None,
            task_type: None,
            description: None,
            instructions: None,
            edit: false,
            agent: None,
            timeout: None,
        }
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
            let template_path = Path::new("templates").join(format!("{}.md", task_type));
            if !ctx.file_system().exists(&template_path).await? {
                return Err(format!(
                    "No template found for task type '{}'. Please check the templates folder.",
                    task_type
                )
                .into());
            }
        }

        // Create task directory
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
        let task_dir_name = format!("{}-{}", timestamp, name);
        let task_dir = Path::new(".tsk/tasks").join(&task_dir_name);
        ctx.file_system().create_dir(&task_dir).await?;

        // Create instructions file
        let instructions_path = self
            .create_instructions_file(&task_dir, &task_type, ctx)
            .await?;

        // Handle edit mode
        if self.edit {
            self.open_editor(&instructions_path)?;
            self.check_instructions_not_empty(&instructions_path, ctx)
                .await?;
        }

        // Create and return the task
        let task = Task::new_with_id(
            task_dir_name.clone(),
            name,
            task_type,
            None, // description is now stored in instructions file
            Some(instructions_path),
            self.agent,
            timeout,
        );

        Ok(task)
    }

    async fn create_instructions_file(
        &self,
        task_dir: &Path,
        task_type: &str,
        ctx: &AppContext,
    ) -> Result<String, Box<dyn Error>> {
        let dest_path = task_dir.join("instructions.md");
        let fs = ctx.file_system();

        if let Some(ref inst_path) = self.instructions {
            // Copy existing instructions file
            let content = fs.read_file(Path::new(inst_path)).await?;
            fs.write_file(&dest_path, &content).await?;
            println!("Copied instructions file to task directory");
            Ok(dest_path.to_string_lossy().to_string())
        } else if let Some(ref desc) = self.description {
            // Check if a template exists for this task type
            let template_path = Path::new("templates").join(format!("{}.md", task_type));
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

            fs.write_file(&dest_path, &content).await?;
            if fs.exists(&template_path).await? {
                println!("Created instructions file from {} template", task_type);
            } else {
                println!("Created instructions file from description");
            }
            Ok(dest_path.to_string_lossy().to_string())
        } else if self.edit {
            // Create empty instructions file for editing
            let template_path = Path::new("templates").join(format!("{}.md", task_type));
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

            fs.write_file(&dest_path, &initial_content).await?;
            println!("Created instructions file for editing");
            Ok(dest_path.to_string_lossy().to_string())
        } else {
            return Err("No description or instructions provided".into());
        }
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
        instructions_path: &str,
        ctx: &AppContext,
    ) -> Result<(), Box<dyn Error>> {
        // Check if file is empty after editing
        let content = ctx
            .file_system()
            .read_file(Path::new(instructions_path))
            .await?;
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
        let template_path = Path::new("templates/feature.md");
        let fs = Arc::new(
            MockFileSystem::new()
                .with_dir(&current_dir.join(".tsk/tasks").to_string_lossy().to_string())
                .with_dir("templates")
                .with_file(
                    &template_path.to_string_lossy().to_string(),
                    template_content,
                ),
        );

        let ctx = AppContext::builder().with_file_system(fs.clone()).build();

        let task = TaskBuilder::new()
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
            .name("test-task".to_string())
            .build(&ctx)
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Either description or instructions"));
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
}
