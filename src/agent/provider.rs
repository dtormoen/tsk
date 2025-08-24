use super::{Agent, ClaudeCodeAgent, NoOpAgent};
use crate::context::tsk_config::TskConfig;
use std::sync::Arc;

/// Provider for creating and managing AI agents
pub struct AgentProvider;

impl AgentProvider {
    /// Get an agent by name with TSK configuration for configuration
    ///
    /// # Arguments
    /// * `name` - The name of the agent to create
    /// * `tsk_config` - TSK configuration containing environment settings
    ///
    /// # Returns
    /// An `Arc<dyn Agent>` for the requested agent type
    pub fn get_agent(name: &str, tsk_config: Arc<TskConfig>) -> anyhow::Result<Arc<dyn Agent>> {
        match name {
            "claude-code" => Ok(Arc::new(ClaudeCodeAgent::with_tsk_config(tsk_config))),
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
    use crate::context::AppContext;

    #[test]
    fn test_agent_provider_get_agent() {
        // Test getting a valid agent
        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = AgentProvider::get_agent("claude-code", tsk_config);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "claude-code");
    }

    #[test]
    fn test_agent_provider_get_invalid_agent() {
        // Test getting an invalid agent
        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = AgentProvider::get_agent("invalid-agent", tsk_config);
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
        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = AgentProvider::get_agent("no-op", tsk_config);
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
