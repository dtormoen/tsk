use super::Command;
use crate::agent::AgentProvider;
use crate::assets::layered::LayeredAssetManager;
use crate::context::AppContext;
use crate::docker::composer::DockerComposer;
use crate::docker::image_manager::DockerImageManager;
use crate::docker::template_manager::DockerTemplateManager;
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::repo_utils::find_repository_root;
use crate::task::Task;
use crate::task_runner::TaskRunner;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;

pub struct DebugCommand {
    pub name: String,
    pub agent: Option<String>,
    pub tech_stack: Option<String>,
    pub project: Option<String>,
}

#[async_trait]
impl Command for DebugCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting debug session: {}", self.name);

        let repo_root = find_repository_root(Path::new("."))?;

        // Auto-detect tech_stack if not provided
        let tech_stack = match &self.tech_stack {
            Some(ts) => {
                println!("Using tech stack: {ts}");
                ts.clone()
            }
            None => match ctx.repository_context().detect_tech_stack(&repo_root).await {
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
            None => {
                match ctx
                    .repository_context()
                    .detect_project_name(&repo_root)
                    .await
                {
                    Ok(detected) => {
                        println!("Auto-detected project name: {detected}");
                        Some(detected)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                        Some("default".to_string())
                    }
                }
            }
        };

        // Create a debug instructions file
        let debug_instructions = format!(
            "# Debug Session: {}\n\nThis is an interactive debug session for exploring and testing.",
            self.name
        );
        let temp_dir = ctx.xdg_directories().runtime_dir().join("tmp");
        ctx.file_system().create_dir(&temp_dir).await?;
        let instructions_file = temp_dir.join(format!("{}-debug.md", self.name));
        ctx.file_system()
            .write_file(&instructions_file, &debug_instructions)
            .await?;

        // Get current commit for the task
        let source_commit = ctx
            .git_operations()
            .get_current_commit(&repo_root)
            .await
            .unwrap_or_else(|_| "HEAD".to_string());

        // Create a minimal task for debug session
        let timestamp = chrono::Local::now();
        let task_id = format!("{}-debug-{}", timestamp.format("%Y-%m-%d-%H%M"), self.name);
        let branch_name = format!("tsk/{task_id}");

        let agent = self
            .agent
            .clone()
            .unwrap_or_else(|| AgentProvider::default_agent().to_string());

        let mut task = Task::new(
            task_id.clone(),
            repo_root.clone(),
            self.name.clone(),
            "debug".to_string(),
            instructions_file.to_string_lossy().to_string(),
            agent,
            0, // No timeout for debug sessions
            branch_name,
            source_commit,
            tech_stack,
            project.unwrap_or_else(|| "default".to_string()),
            chrono::Local::now(),
            repo_root.clone(), // temporary, will be updated after repo copy
        );

        let repo_manager = RepoManager::new(
            ctx.xdg_directories(),
            ctx.file_system(),
            ctx.git_operations(),
        );

        // Copy the repository for the debug task
        let (copied_repo_path, _) = repo_manager
            .copy_repo(&task_id, &repo_root, Some(&task.source_commit))
            .await
            .map_err(|e| format!("Failed to copy repository: {e}"))?;

        // Update the task with the copied repository path
        task.copied_repo_path = copied_repo_path;
        let docker_manager = DockerManager::new(ctx.docker_client(), ctx.file_system());

        // Create image manager on-demand for the task's repository
        let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
            Some(&repo_root),
            &ctx.xdg_directories(),
        ));
        let template_manager =
            DockerTemplateManager::new(asset_manager.clone(), ctx.xdg_directories());
        let composer = DockerComposer::new(DockerTemplateManager::new(
            asset_manager,
            ctx.xdg_directories(),
        ));
        let image_manager = Arc::new(DockerImageManager::new(
            ctx.docker_client(),
            template_manager,
            composer,
        ));

        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            image_manager,
            ctx.file_system(),
            ctx.notification_client(),
        );

        // Execute task in interactive mode
        task_runner
            .execute_task(&task, true)
            .await
            .map_err(|e| e.message)?;

        // Clean up the temporary instructions file
        let _ = ctx.file_system().remove_file(&instructions_file).await;

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
        };

        // Verify the command has the expected fields
        assert_eq!(cmd.name, "test-debug");
        assert_eq!(cmd.agent, Some("claude_code".to_string()));
        assert_eq!(cmd.tech_stack, None);
        assert_eq!(cmd.project, None);
    }
}
