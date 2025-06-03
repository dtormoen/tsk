use super::Command;
use crate::docker::get_docker_manager;
use async_trait::async_trait;
use std::error::Error;

pub struct StopProxyCommand;

#[async_trait]
impl Command for StopProxyCommand {
    async fn execute(&self) -> Result<(), Box<dyn Error>> {
        println!("Stopping TSK proxy container...");

        let docker_manager = get_docker_manager()?;
        docker_manager.stop_proxy().await?;

        println!("Proxy container stopped successfully");
        Ok(())
    }
}
