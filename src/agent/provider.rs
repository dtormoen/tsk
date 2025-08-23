use super::{Agent, ClaudeCodeAgent, NoOpAgent};
use crate::context::config::Config;
use std::sync::Arc;

/// Provider for creating and managing AI agents
pub struct AgentProvider;

impl AgentProvider {
    /// Get an agent by name with a specific configuration
    ///
    /// # Arguments
    /// * `name` - The name of the agent to create
    /// * `config` - Configuration containing environment-dependent settings
    ///
    /// # Returns
    /// An `Arc<dyn Agent>` for the requested agent type
    pub fn get_agent(name: &str, config: Arc<Config>) -> anyhow::Result<Arc<dyn Agent>> {
        match name {
            "claude-code" => Ok(Arc::new(ClaudeCodeAgent::with_config(config))),
            "no-op" => Ok(Arc::new(NoOpAgent)),
            _ => Err(anyhow::anyhow!("Unknown agent: {}", name)),
        }
    }

    /// List all available agents
    pub fn list_agents() -> Vec<&'static str> {
        vec!["claude-code", "no-op"]
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
        let config = Arc::new(Config::new());
        let agent = AgentProvider::get_agent("claude-code", config);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "claude-code");
    }

    #[test]
    fn test_agent_provider_get_invalid_agent() {
        // Test getting an invalid agent
        let config = Arc::new(Config::new());
        let agent = AgentProvider::get_agent("invalid-agent", config);
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
        assert!(AgentProvider::is_valid_agent("no-op"));
        assert!(!AgentProvider::is_valid_agent("invalid-agent"));
    }

    #[test]
    fn test_agent_provider_get_no_op_agent() {
        // Test getting the no-op agent
        let config = Arc::new(Config::new());
        let agent = AgentProvider::get_agent("no-op", config);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "no-op");
    }

    #[test]
    fn test_agent_provider_list_includes_no_op() {
        let agents = AgentProvider::list_agents();
        assert!(agents.contains(&"no-op"));
    }
}
