use super::{Agent, ClaudeCodeAgent};
use std::sync::Arc;

/// Provider for creating and managing AI agents
pub struct AgentProvider;

impl AgentProvider {
    /// Get an agent by name
    pub fn get_agent(name: &str) -> anyhow::Result<Arc<dyn Agent>> {
        match name {
            "claude-code" => Ok(Arc::new(ClaudeCodeAgent::new())),
            _ => Err(anyhow::anyhow!("Unknown agent: {}", name)),
        }
    }

    /// List all available agents
    pub fn list_agents() -> Vec<&'static str> {
        vec!["claude-code"]
    }

    /// Get the default agent name
    pub fn default_agent() -> &'static str {
        "claude-code"
    }

    /// Validate an agent name
    pub fn is_valid_agent(name: &str) -> bool {
        Self::list_agents().contains(&name)
    }
}
