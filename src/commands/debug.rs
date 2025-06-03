use super::Command;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct DebugCommand {
    pub name: String,
}

#[async_trait]
impl Command for DebugCommand {
    async fn execute(&self) -> Result<(), Box<dyn Error>> {
        println!("Starting debug session: {}", self.name);

        let task_manager = TaskManager::new()?;
        task_manager
            .run_debug_container(&self.name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}
