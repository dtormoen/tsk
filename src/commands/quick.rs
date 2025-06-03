use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct QuickCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
}

impl QuickCommand {
    fn validate_input(&self) -> Result<(), Box<dyn Error>> {
        if self.description.is_none() && self.instructions.is_none() && !self.edit {
            return Err("Either --description or --instructions must be provided, or use --edit to create instructions interactively".into());
        }
        Ok(())
    }

    fn validate_task_type(&self) -> Result<(), Box<dyn Error>> {
        if self.r#type != "generic" {
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
            if !template_path.exists() {
                return Err(format!(
                    "No template found for task type '{}'. Please check the templates folder.",
                    self.r#type
                )
                .into());
            }
        }
        Ok(())
    }

    fn create_task_directory(&self) -> Result<std::path::PathBuf, Box<dyn Error>> {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M");
        let task_dir_name = format!("{}-{}", timestamp, self.name);
        let task_dir = Path::new(".tsk/quick-tasks").join(&task_dir_name);
        std::fs::create_dir_all(&task_dir)?;
        Ok(task_dir)
    }

    fn create_instructions_file(&self, task_dir: &Path) -> Result<String, Box<dyn Error>> {
        let dest_path = task_dir.join("instructions.md");

        if let Some(ref inst_path) = self.instructions {
            // Copy existing instructions file
            let content = std::fs::read_to_string(inst_path)?;
            std::fs::write(&dest_path, content)?;
            println!("Copied instructions file to task directory");
            Ok(dest_path.to_string_lossy().to_string())
        } else if let Some(ref desc) = self.description {
            // Check if a template exists for this task type
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
            let content = if template_path.exists() {
                match std::fs::read_to_string(&template_path) {
                    Ok(template_content) => template_content.replace("{{DESCRIPTION}}", desc),
                    Err(e) => {
                        eprintln!("Warning: Failed to read template file: {}", e);
                        desc.clone()
                    }
                }
            } else {
                desc.clone()
            };

            std::fs::write(&dest_path, content)?;
            if template_path.exists() {
                println!("Created instructions file from {} template", self.r#type);
            } else {
                println!("Created instructions file from description");
            }
            Ok(dest_path.to_string_lossy().to_string())
        } else if self.edit {
            // Create empty instructions file for editing
            let template_path = Path::new("templates").join(format!("{}.md", self.r#type));
            let initial_content = if template_path.exists() {
                match std::fs::read_to_string(&template_path) {
                    Ok(template_content) => template_content.replace(
                        "{{DESCRIPTION}}",
                        "<!-- TODO: Add your task description here -->",
                    ),
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            };

            std::fs::write(&dest_path, initial_content)?;
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

        // Check if file is empty after editing
        let content = std::fs::read_to_string(instructions_path)?;
        if content.trim().is_empty() {
            return Err("Instructions file is empty. Task execution cancelled.".into());
        }

        Ok(())
    }
}

#[async_trait]
impl Command for QuickCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Executing quick task: {}", self.name);
        println!("Type: {}", self.r#type);

        self.validate_input()?;
        self.validate_task_type()?;

        let task_dir = self.create_task_directory()?;
        let instructions_path = self.create_instructions_file(&task_dir)?;

        if self.edit {
            if let Err(e) = self.open_editor(&instructions_path) {
                // Clean up on error
                let _ = std::fs::remove_dir_all(&task_dir);
                return Err(e);
            }
        }

        if let Some(ref agent) = self.agent {
            println!("Agent: {}", agent);
        }
        println!("Timeout: {} minutes", self.timeout);

        let task_manager = TaskManager::new(ctx)?;
        task_manager
            .execute_task(&self.name, None, Some(&instructions_path))
            .await
            .map_err(|e| e.message)?;

        Ok(())
    }
}
