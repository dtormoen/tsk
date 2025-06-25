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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_provider_get_agent() {
        // Test getting a valid agent
        let agent = AgentProvider::get_agent("claude-code");
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "claude-code");
    }

    #[test]
    fn test_agent_provider_get_invalid_agent() {
        // Test getting an invalid agent
        let agent = AgentProvider::get_agent("invalid-agent");
        assert!(agent.is_err());
        let err = agent.err().unwrap();
        assert!(err.to_string().contains("Unknown agent"));
    }

    #[test]
    fn test_agent_provider_list_agents() {
        let agents = AgentProvider::list_agents();
        assert!(!agents.is_empty());
        assert!(agents.contains(&"claude-code"));
    }

    #[test]
    fn test_agent_provider_default_agent() {
        assert_eq!(AgentProvider::default_agent(), "claude-code");
    }

    #[test]
    fn test_agent_provider_is_valid_agent() {
        assert!(AgentProvider::is_valid_agent("claude-code"));
        assert!(!AgentProvider::is_valid_agent("invalid-agent"));
    }
}
