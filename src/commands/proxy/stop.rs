use crate::commands::Command;
use crate::context::AppContext;
use crate::docker::proxy_manager::ProxyManager;
use async_trait::async_trait;
use std::error::Error;

pub struct ProxyStopCommand;

#[async_trait]
impl Command for ProxyStopCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let proxy_manager = ProxyManager::new(ctx);
        proxy_manager.stop_proxy().await?;
        Ok(())
    }
}
