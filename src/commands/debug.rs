use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Command for starting interactive debug sessions.
///
/// Creates a task configured for interactive debugging, allowing
/// developers to explore and test in isolated Docker containers.
/// Accepts the same arguments as the quick command but always runs interactively.
pub struct DebugCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub timeout: u32,
    pub tech_stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
}

#[async_trait]
impl Command for DebugCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting debug session: {}", self.name);
        println!("Type: {}", self.r#type);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create task using TaskBuilder, same as quick command but always interactive
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(self.description.clone())
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(self.agent.clone())
            .timeout(self.timeout)
            .tech_stack(self.tech_stack.clone())
            .project(self.project.clone())
            .with_interactive(true) // Debug sessions are always interactive
            .build(ctx)
            .await?;

        if let Some(ref agent) = self.agent {
            println!("Agent: {agent}");
        }
        println!("Timeout: {} minutes (0 = no timeout)", self.timeout);

        // Update terminal title for the debug session
        ctx.terminal_operations()
            .set_title(&format!("TSK Debug: {}", self.name));

        // Execute the task using TaskManager
        let task_manager = TaskManager::new(ctx)?;
        let result = task_manager
            .execute_queued_task(&task)
            .await
            .map_err(|e| e.message);

        // Restore terminal title
        ctx.terminal_operations().restore_title();

        result?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_command_structure() {
        let cmd = DebugCommand {
            name: "test-debug".to_string(),
            r#type: "generic".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: Some("claude-code".to_string()),
            timeout: 0,
            tech_stack: None,
            project: None,
            repo: None,
        };

        // Verify the command has the expected fields
        assert_eq!(cmd.name, "test-debug");
        assert_eq!(cmd.r#type, "generic");
        assert_eq!(cmd.description, Some("Test description".to_string()));
        assert_eq!(cmd.prompt, None);
        assert!(!cmd.edit);
        assert_eq!(cmd.agent, Some("claude-code".to_string()));
        assert_eq!(cmd.timeout, 0);
        assert_eq!(cmd.tech_stack, None);
        assert_eq!(cmd.project, None);
        assert_eq!(cmd.repo, None);
    }
}
