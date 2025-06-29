use super::Command;
use crate::context::AppContext;
use crate::docker::DockerManager;
use async_trait::async_trait;
use std::error::Error;

pub struct StopProxyCommand;

#[async_trait]
impl Command for StopProxyCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Stopping TSK proxy container...");

        let docker_manager = DockerManager::new(ctx.docker_client(), ctx.file_system());
        docker_manager.stop_proxy().await?;

        println!("Proxy container stopped successfully");
        Ok(())
    }
}
