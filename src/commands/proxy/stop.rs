use crate::commands::Command;
use crate::context::AppContext;
use crate::docker::DockerManager;
use async_trait::async_trait;
use std::error::Error;

pub struct ProxyStopCommand;

#[async_trait]
impl Command for ProxyStopCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Stopping TSK proxy container...");

        let docker_manager = DockerManager::new(ctx.docker_client());
        docker_manager.stop_proxy().await?;

        println!("Proxy container stopped successfully");
        Ok(())
    }
}
