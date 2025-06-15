use super::Command;
use crate::client::TskClient;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::task::TaskBuilder;
use crate::task_storage::get_task_storage;
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

#[async_trait]
impl Command for AddCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Adding task to queue: {}", self.name);

        // Find repository root
        let repo_root = find_repository_root(Path::new("."))?;

        // Create task using TaskBuilder
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(self.description.clone())
            .instructions(self.instructions.clone())
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .build(ctx)
            .await?;

        // Try to add task via server first
        let client = TskClient::new(ctx.xdg_directories());

        if client.is_server_available().await {
            // Server is available, use it
            match client.add_task(repo_root.clone(), task.clone()).await {
                Ok(_) => {
                    println!("Task added via server");
                }
                Err(_) => {
                    eprintln!("Failed to add task via server");
                    eprintln!("Falling back to direct file write...");

                    // Fall back to direct storage
                    let storage = get_task_storage(ctx.xdg_directories(), ctx.file_system());
                    storage
                        .add_task(task.clone())
                        .await
                        .map_err(|e| e as Box<dyn Error>)?;
                }
            }
        } else {
            // Server not available, write directly
            let storage = get_task_storage(ctx.xdg_directories(), ctx.file_system());
            storage
                .add_task(task.clone())
                .await
                .map_err(|e| e as Box<dyn Error>)?;
        }

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
