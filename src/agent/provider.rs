use super::{Agent, ClaudeAgent, CodexAgent, IntegAgent, NoOpAgent};
use crate::context::tsk_env::TskEnv;
use std::sync::Arc;

/// Provider for creating and managing AI agents
pub struct AgentProvider;

impl AgentProvider {
    /// Get an agent by name with TSK environment for configuration
    ///
    /// # Arguments
    /// * `name` - The name of the agent to create
    /// * `tsk_env` - TSK environment containing environment settings
    ///
    /// # Returns
    /// An `Arc<dyn Agent>` for the requested agent type
    pub fn get_agent(name: &str, tsk_env: Arc<TskEnv>) -> anyhow::Result<Arc<dyn Agent>> {
        match name {
            "claude" => Ok(Arc::new(ClaudeAgent::with_tsk_env(tsk_env))),
            "codex" => Ok(Arc::new(CodexAgent::with_tsk_env(tsk_env))),
            "no-op" => Ok(Arc::new(NoOpAgent)),
            "integ" => Ok(Arc::new(IntegAgent)),
            _ => Err(anyhow::anyhow!("Unknown agent: {}", name)),
        }
    }

    /// List user-facing agents
    pub fn list_agents() -> Vec<&'static str> {
        vec!["claude", "codex", "no-op"]
    }

    /// Get the default agent name
    pub fn default_agent() -> &'static str {
        "claude"
    }

    /// Validate an agent name (includes internal agents like integ)
    pub fn is_valid_agent(name: &str) -> bool {
        Self::list_agents().contains(&name) || name == "integ"
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
        let tsk_env = app_context.tsk_env();
        let agent = AgentProvider::get_agent("claude", tsk_env);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "claude");
    }

    #[test]
    fn test_agent_provider_get_invalid_agent() {
        // Test getting an invalid agent
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = AgentProvider::get_agent("invalid-agent", tsk_env);
        assert!(agent.is_err());
        let err = agent.err().unwrap();
        assert!(err.to_string().contains("Unknown agent"));
    }

    #[test]
    fn test_agent_provider_list_agents() {
        let agents = AgentProvider::list_agents();
        assert!(!agents.is_empty());
        assert!(agents.contains(&"claude"));
    }

    #[test]
    fn test_agent_provider_default_agent() {
        assert_eq!(AgentProvider::default_agent(), "claude");
    }

    #[test]
    fn test_agent_provider_is_valid_agent() {
        assert!(AgentProvider::is_valid_agent("claude"));
        assert!(AgentProvider::is_valid_agent("codex"));
        assert!(AgentProvider::is_valid_agent("no-op"));
        assert!(!AgentProvider::is_valid_agent("invalid-agent"));
    }

    #[test]
    fn test_agent_provider_get_codex_agent() {
        // Test getting the codex agent
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = AgentProvider::get_agent("codex", tsk_env);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "codex");
    }

    #[test]
    fn test_agent_provider_get_no_op_agent() {
        // Test getting the no-op agent
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = AgentProvider::get_agent("no-op", tsk_env);
        assert!(agent.is_ok());
        let agent = agent.unwrap();
        assert_eq!(agent.name(), "no-op");
    }

    #[test]
    fn test_agent_provider_list_includes_codex() {
        let agents = AgentProvider::list_agents();
        assert!(agents.contains(&"codex"));
    }

    #[test]
    fn test_agent_provider_list_includes_no_op() {
        let agents = AgentProvider::list_agents();
        assert!(agents.contains(&"no-op"));
    }
}
