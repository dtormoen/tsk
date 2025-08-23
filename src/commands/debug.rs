use super::Command;
use crate::agent::AgentProvider;
use crate::context::AppContext;
use crate::git::RepoManager;
use crate::repo_utils::find_repository_root;
use crate::task::Task;
use crate::task_runner::TaskRunner;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

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

        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Auto-detect tech_stack if not provided
        let tech_stack = match &self.tech_stack {
            Some(ts) => {
                println!("Using tech stack: {ts}");
                ts.clone()
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

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => {
                println!("Using project: {p}");
                Some(p.clone())
            }
            None => match crate::repository::detect_project_name(&repo_root).await {
                Ok(detected) => {
                    println!("Auto-detected project name: {detected}");
                    Some(detected)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                    Some("default".to_string())
                }
            },
        };

        // Create a debug prompt file
        let debug_prompt = format!(
            "# Debug Session: {}\n\nThis is an interactive debug session for exploring and testing.",
            self.name
        );
        let temp_dir = ctx.xdg_directories().runtime_dir().join("tmp");
        ctx.file_system().create_dir(&temp_dir).await?;
        let prompt_file = temp_dir.join(format!("{}-debug.md", self.name));
        ctx.file_system()
            .write_file(&prompt_file, &debug_prompt)
            .await?;

        // Get current commit for the task
        let source_commit = ctx
            .git_operations()
            .get_current_commit(&repo_root)
            .await
            .unwrap_or_else(|_| "HEAD".to_string());

        // Create a minimal task for debug session
        let task_id = nanoid::nanoid!(8);
        let sanitized_name = crate::utils::sanitize_for_branch_name(&self.name);
        let branch_name = format!("tsk/debug/{sanitized_name}/{task_id}");

        let agent = self
            .agent
            .clone()
            .unwrap_or_else(|| AgentProvider::default_agent().to_string());

        let mut task = Task::new(
            task_id.clone(),
            repo_root.clone(),
            self.name.clone(),
            "debug".to_string(),
            prompt_file.to_string_lossy().to_string(),
            agent,
            0, // No timeout for debug sessions
            branch_name.clone(),
            source_commit,
            tech_stack,
            project.unwrap_or_else(|| "default".to_string()),
            chrono::Local::now(),
            repo_root.clone(), // temporary, will be updated after repo copy
            true,              // Debug sessions are always interactive
        );

        // Create a RepoManager to handle repository operations
        let repo_manager = RepoManager::new(
            ctx.xdg_directories(),
            ctx.file_system(),
            ctx.git_operations(),
            ctx.git_sync_manager(),
        );

        // Copy the repository for the debug task
        let (copied_repo_path, _) = repo_manager
            .copy_repo(
                &task_id,
                &repo_root,
                Some(&task.source_commit),
                &branch_name,
            )
            .await
            .map_err(|e| format!("Failed to copy repository: {e}"))?;

        // Update the task with the copied repository path
        task.copied_repo_path = copied_repo_path;

        // Create TaskRunner using AppContext
        let task_runner = TaskRunner::new(ctx);

        // Execute task (interactive mode is set on the task itself)
        task_runner
            .execute_task(&task)
            .await
            .map_err(|e| e.message)?;

        // Clean up the temporary prompt file
        let _ = ctx.file_system().remove_file(&prompt_file).await;

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
