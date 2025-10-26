use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::stdin_utils::{merge_description_with_stdin, read_piped_input};
use crate::task::TaskBuilder;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Command for starting interactive shell sessions.
///
/// Creates a sandbox container with an agent for interactive use.
/// Allows developers to explore and test in isolated Docker containers.
/// All flags have sensible defaults for convenience.
pub struct ShellCommand {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
}

#[async_trait]
impl Command for ShellCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting shell session: {}", self.name);
        println!("Type: {}", self.r#type);

        // Read from stdin if data is piped
        let piped_input = read_piped_input()?;

        // Merge piped input with CLI description (piped input takes precedence)
        let final_description = merge_description_with_stdin(self.description.clone(), piped_input);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create task using TaskBuilder, always interactive
        let task = TaskBuilder::new()
            .repo_root(repo_root.clone())
            .name(self.name.clone())
            .task_type(self.r#type.clone())
            .description(final_description)
            .instructions_file(self.prompt.as_ref().map(PathBuf::from))
            .edit(self.edit)
            .agent(self.agent.clone())
            .stack(self.stack.clone())
            .project(self.project.clone())
            .with_interactive(true) // Shell sessions are always interactive
            .build(ctx)
            .await?;

        if let Some(ref agent) = self.agent {
            println!("Agent: {agent}");
        }

        // Update terminal title for the shell session
        ctx.terminal_operations()
            .set_title(&format!("TSK Shell: {}", self.name));

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
    fn test_shell_command_structure() {
        let cmd = ShellCommand {
            name: "test-shell".to_string(),
            r#type: "shell".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: Some("claude".to_string()),
            stack: None,
            project: None,
            repo: None,
        };

        // Verify the command has the expected fields
        assert_eq!(cmd.name, "test-shell");
        assert_eq!(cmd.r#type, "shell");
        assert_eq!(cmd.description, Some("Test description".to_string()));
        assert_eq!(cmd.prompt, None);
        assert!(!cmd.edit);
        assert_eq!(cmd.agent, Some("claude".to_string()));
        assert_eq!(cmd.stack, None);
        assert_eq!(cmd.project, None);
        assert_eq!(cmd.repo, None);
    }
}
