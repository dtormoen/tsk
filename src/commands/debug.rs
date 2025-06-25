use super::Command;
use crate::context::AppContext;
use crate::docker::DockerManager;
use crate::git::RepoManager;
use crate::repo_utils::find_repository_root;
use crate::task_runner::TaskRunner;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

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
                println!("Using tech stack: {}", ts);
                ts.clone()
            }
            None => match ctx.repository_context().detect_tech_stack(&repo_root).await {
                Ok(detected) => {
                    println!("Auto-detected tech stack: {}", detected);
                    detected
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to detect tech stack: {}. Using default.",
                        e
                    );
                    "default".to_string()
                }
            },
        };

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => {
                println!("Using project: {}", p);
                Some(p.clone())
            }
            None => {
                match ctx
                    .repository_context()
                    .detect_project_name(&repo_root)
                    .await
                {
                    Ok(detected) => {
                        println!("Auto-detected project name: {}", detected);
                        Some(detected)
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to detect project name: {}. Using default.",
                            e
                        );
                        Some("default".to_string())
                    }
                }
            }
        };

        let repo_manager = RepoManager::new(
            ctx.xdg_directories(),
            ctx.file_system(),
            ctx.git_operations(),
        );
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            ctx.docker_image_manager(),
            ctx.file_system(),
            ctx.notification_client(),
        );

        task_runner
            .run_debug_container(
                &self.name,
                self.agent.as_deref(),
                &repo_root,
                &tech_stack,
                project.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;

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
