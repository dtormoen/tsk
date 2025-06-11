use super::*;
use crate::context::file_system::tests::MockFileSystem;
use std::path::Path;
use std::sync::Arc;

#[test]
fn test_agent_provider_get_agent() {
    // Test getting a valid agent
    let agent = AgentProvider::get_agent("claude-code");
    assert!(agent.is_ok());
    let agent = agent.unwrap();
    assert_eq!(agent.name(), "claude-code");
    assert_eq!(agent.docker_image(), "tsk/base");
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

#[test]
fn test_claude_code_agent_properties() {
    let agent = ClaudeCodeAgent::new();

    // Test docker image
    assert_eq!(agent.docker_image(), "tsk/base");

    // Test name
    assert_eq!(agent.name(), "claude-code");

    // Test volumes
    let volumes = agent.volumes();
    assert_eq!(volumes.len(), 2);

    // Should mount .claude directory and .claude.json file
    let volume_paths: Vec<&str> = volumes
        .iter()
        .map(|(_, container_path, _)| container_path.as_str())
        .collect();
    assert!(volume_paths.contains(&"/home/agent/.claude"));
    assert!(volume_paths.contains(&"/home/agent/.claude.json"));

    // Test environment variables
    let env = agent.environment();
    assert_eq!(env.len(), 2);

    let env_map: std::collections::HashMap<_, _> = env.into_iter().collect();
    assert_eq!(env_map.get("HOME"), Some(&"/home/agent".to_string()));
    assert_eq!(env_map.get("USER"), Some(&"agent".to_string()));
}

#[test]
fn test_claude_code_agent_build_command() {
    let agent = ClaudeCodeAgent::new();

    // Test with full path
    let command = agent.build_command("/tmp/instructions.md");
    assert_eq!(command.len(), 3);
    assert_eq!(command[0], "sh");
    assert_eq!(command[1], "-c");
    assert!(command[2].contains("cat /instructions/instructions.md"));
    assert!(command[2].contains("claude -p --verbose --output-format stream-json"));

    // Test with complex path
    let command = agent.build_command("/path/to/task/instructions.txt");
    assert!(command[2].contains("cat /instructions/instructions.txt"));
}

#[tokio::test]
async fn test_claude_code_agent_validate_without_config() {
    // Create a temporary HOME directory without .claude.json
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", temp_dir.path());

    let agent = ClaudeCodeAgent::new();
    let result = agent.validate().await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("Claude configuration not found"));
}

#[test]
fn test_claude_code_agent_create_log_processor() {
    let fs = Arc::new(MockFileSystem::new());
    let agent = ClaudeCodeAgent::new();

    let log_processor = agent.create_log_processor(fs);

    // Just verify we can create a log processor
    // The actual log processor functionality is tested elsewhere
    let _ = log_processor.get_full_log();
}

/// Test agent for testing purposes
#[allow(dead_code)]
struct TestAgent {
    name: String,
    docker_image: String,
}

#[allow(dead_code)]
impl TestAgent {
    fn new(name: &str, docker_image: &str) -> Self {
        Self {
            name: name.to_string(),
            docker_image: docker_image.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Agent for TestAgent {
    fn docker_image(&self) -> &str {
        &self.docker_image
    }

    fn build_command(&self, instruction_path: &str) -> Vec<String> {
        vec!["test".to_string(), instruction_path.to_string()]
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        vec![("/test".to_string(), "/test".to_string(), ":ro".to_string())]
    }

    fn environment(&self) -> Vec<(String, String)> {
        vec![("TEST_VAR".to_string(), "test_value".to_string())]
    }

    fn create_log_processor(
        &self,
        _file_system: Arc<dyn crate::context::file_system::FileSystemOperations>,
    ) -> Box<dyn LogProcessor> {
        struct TestLogProcessor;

        #[async_trait::async_trait]
        impl LogProcessor for TestLogProcessor {
            fn process_line(&mut self, _line: &str) -> Option<String> {
                Some("test".to_string())
            }

            fn get_full_log(&self) -> String {
                "test log".to_string()
            }

            async fn save_full_log(&self, _path: &Path) -> Result<(), String> {
                Ok(())
            }

            fn get_final_result(&self) -> Option<&crate::log_processor::TaskResult> {
                None
            }
        }

        Box::new(TestLogProcessor)
    }

    fn name(&self) -> &str {
        &self.name
    }
}
