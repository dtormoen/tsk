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
}

#[async_trait]
impl Command for DebugCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting debug session: {}", self.name);

        let repo_manager = RepoManager::new(ctx.file_system(), ctx.git_operations());
        let docker_manager = DockerManager::new(ctx.docker_client());
        let task_runner = TaskRunner::new(
            repo_manager,
            docker_manager,
            ctx.file_system(),
            ctx.notification_client(),
        );

        let repo_root = find_repository_root(Path::new("."))?;

        task_runner
            .run_debug_container(&self.name, self.agent.as_deref(), &repo_root)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}
