use super::Command;
use crate::context::AppContext;
use crate::task::{get_task_storage, Task};
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct AddCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
}

impl AddCommand {
    fn validate_input(&self) -> Result<(), Box<dyn Error>> {
        if self.description.is_none() && self.instructions.is_none() && !self.edit {
            return Err("Either --description or --instructions must be provided, or use --edit to create instructions interactively".into());
        }
        Ok(())
    }

    async fn validate_task_type(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.r#type != "generic" {
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
            if !ctx.file_system().exists(&template_path).await? {
                return Err(format!(
                    "No template found for task type '{}'. Please check the templates folder.",
                    self.r#type
                )
                .into());
            }
        }
        Ok(())
    }

    async fn create_task_directory(
        &self,
        ctx: &AppContext,
    ) -> Result<std::path::PathBuf, Box<dyn Error>> {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
        let task_dir_name = format!("{}-{}", timestamp, self.name);
        let task_dir = Path::new(".tsk/tasks").join(&task_dir_name);
        ctx.file_system().create_dir(&task_dir).await?;
        Ok(task_dir)
    }

    async fn create_instructions_file(
        &self,
        task_dir: &Path,
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
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
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
                println!("Created instructions file from {} template", self.r#type);
            } else {
                println!("Created instructions file from description");
            }
            Ok(dest_path.to_string_lossy().to_string())
        } else if self.edit {
            // Create empty instructions file for editing
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
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

#[async_trait]
impl Command for AddCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Adding task to queue: {}", self.name);

        self.validate_input()?;
        self.validate_task_type(ctx).await?;

        let task_dir = self.create_task_directory(ctx).await?;
        let task_dir_name = task_dir.file_name().unwrap().to_string_lossy().to_string();

        let instructions_path = self.create_instructions_file(&task_dir, ctx).await?;

        if self.edit {
            self.open_editor(&instructions_path)?;
            self.check_instructions_not_empty(&instructions_path, ctx)
                .await?;
        }

        // Create and save the task
        let task = Task::new_with_id(
            task_dir_name.clone(),
            self.name.clone(),
            self.r#type.clone(),
            None, // description is now stored in instructions file
            Some(instructions_path),
            self.agent.clone(),
            self.timeout,
        );

        let storage = get_task_storage(ctx.file_system());
        storage.add_task(task.clone()).await?;

        println!("\nTask successfully added to queue!");
        println!("Task ID: {}", task.id);
        println!("Type: {}", self.r#type);
        if let Some(ref desc) = self.description {
            println!("Description: {}", desc);
        }
        if self.instructions.is_some() {
            println!("Instructions: Copied to task directory");
        }
        if let Some(ref agent) = self.agent {
            println!("Agent: {}", agent);
        }
        println!("Timeout: {} minutes", self.timeout);
        println!("\nUse 'tsk list' to view all queued tasks");
        println!("Use 'tsk run' to execute the next task in the queue");

        Ok(())
    }
}
