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
/// Creates a minimal task configured for interactive debugging, allowing
/// developers to explore and test in isolated Docker containers.
pub struct DebugCommand {
    pub name: String,
    pub agent: Option<String>,
    pub tech_stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
}

#[async_trait]
impl Command for DebugCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting debug session: {}", self.name);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create a debug prompt as a simple description
        let debug_description = format!(
            "Debug Session: {}\n\nThis is an interactive debug session for exploring and testing.",
            self.name
        );

        // Create task using TaskBuilder similar to quick command
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type("debug".to_string())
            .description(Some(debug_description))
            .instructions_file(None::<PathBuf>) // No external instructions file for debug
            .edit(false) // No need to edit for debug sessions
            .agent(self.agent.clone())
            .timeout(0) // No timeout for debug sessions
            .tech_stack(self.tech_stack.clone())
            .project(self.project.clone())
            .with_interactive(true) // Debug sessions are always interactive
            .build(ctx)
            .await?;

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
            agent: Some("claude_code".to_string()),
            tech_stack: None,
            project: None,
            repo: None,
        };

        // Verify the command has the expected fields
        assert_eq!(cmd.name, "test-debug");
        assert_eq!(cmd.agent, Some("claude_code".to_string()));
        assert_eq!(cmd.tech_stack, None);
        assert_eq!(cmd.project, None);
        assert_eq!(cmd.repo, None);
    }
}
