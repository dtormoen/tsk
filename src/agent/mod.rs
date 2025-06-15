use async_trait::async_trait;
use std::sync::Arc;

mod claude_code;
mod log_processor;
mod provider;

#[cfg(test)]
mod tests;

pub use self::log_processor::LogProcessor;
pub use claude_code::{ClaudeCodeAgent, TaskResult};
pub use provider::AgentProvider;

/// Trait defining the interface for AI agents that can execute tasks
#[async_trait]
pub trait Agent: Send + Sync {
    /// Returns the Docker image name for this agent
    fn docker_image(&self) -> &str;

    /// Returns the command to execute the agent with the given instruction file
    fn build_command(&self, instruction_path: &str) -> Vec<String>;

    /// Returns the volumes to mount for this agent
    /// Format: Vec<(host_path, container_path, options)> where options is like ":ro" for read-only
    fn volumes(&self) -> Vec<(String, String, String)>;

    /// Returns environment variables for this agent
    fn environment(&self) -> Vec<(String, String)>;

    /// Creates a log processor for this agent's output
    fn create_log_processor(
        &self,
        file_system: Arc<dyn crate::context::file_system::FileSystemOperations>,
    ) -> Box<dyn LogProcessor>;

    /// Returns the agent's unique identifier
    fn name(&self) -> &str;

    /// Validates that this agent is properly configured
    async fn validate(&self) -> Result<(), String> {
        Ok(())
    }

    /// Performs any necessary warmup steps before launching the Docker container
    ///
    /// This method is called after validation but before container creation.
    /// It can be used to execute host-side setup commands, refresh credentials,
    /// or perform any other preparatory work needed by the agent.
    ///
    /// The default implementation does nothing, allowing backward compatibility.
    async fn warmup(&self) -> Result<(), String> {
        Ok(())
    }
}
