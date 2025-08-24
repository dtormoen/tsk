use async_trait::async_trait;

mod claude_code;
mod log_processor;
mod no_op;
mod no_op_log_processor;
mod provider;
mod task_result;

pub use self::log_processor::LogProcessor;
pub use self::task_result::TaskResult;
pub use claude_code::ClaudeCodeAgent;
pub use no_op::NoOpAgent;
pub use provider::AgentProvider;

/// Trait defining the interface for AI agents that can execute tasks
#[async_trait]
pub trait Agent: Send + Sync {
    /// Returns the command to execute the agent with the given instruction file
    fn build_command(&self, instruction_path: &str) -> Vec<String>;

    /// Returns the volumes to mount for this agent
    /// Format: Vec<(host_path, container_path, options)> where options is like ":ro" for read-only
    fn volumes(&self) -> Vec<(String, String, String)>;

    /// Returns environment variables for this agent
    fn environment(&self) -> Vec<(String, String)>;

    /// Creates a log processor for this agent's output
    fn create_log_processor(&self) -> Box<dyn LogProcessor>;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test agent for testing purposes
    struct TestAgent {
        name: String,
    }

    impl TestAgent {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl Agent for TestAgent {
        fn build_command(&self, instruction_path: &str) -> Vec<String> {
            vec!["test".to_string(), instruction_path.to_string()]
        }

        fn volumes(&self) -> Vec<(String, String, String)> {
            vec![("/test".to_string(), "/test".to_string(), ":ro".to_string())]
        }

        fn environment(&self) -> Vec<(String, String)> {
            vec![("TEST_VAR".to_string(), "test_value".to_string())]
        }

        fn create_log_processor(&self) -> Box<dyn LogProcessor> {
            struct TestLogProcessor;

            #[async_trait]
            impl LogProcessor for TestLogProcessor {
                fn process_line(&mut self, _line: &str) -> Option<String> {
                    Some("test".to_string())
                }

                fn get_full_log(&self) -> String {
                    "test log".to_string()
                }

                fn get_final_result(&self) -> Option<&TaskResult> {
                    None
                }
            }

            Box::new(TestLogProcessor)
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_agent_trait_is_object_safe() {
        // This test ensures that the Agent trait can be used as a trait object
        fn _assert_object_safe(_: &dyn Agent) {}

        let agent = TestAgent::new("test");
        _assert_object_safe(&agent);
    }

    #[tokio::test]
    async fn test_no_op_agent() {
        let agent = NoOpAgent;

        // Test agent name
        assert_eq!(agent.name(), "no-op");

        // Test build_command
        let command = agent.build_command("/instructions/test.md");
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat '/instructions/test.md'"));

        // Test volumes
        let volumes = agent.volumes();
        assert!(volumes.is_empty());

        // Test environment
        let env = agent.environment();
        assert!(env.is_empty());

        // Test validation (should always succeed)
        assert!(agent.validate().await.is_ok());

        // Test warmup (should always succeed)
        assert!(agent.warmup().await.is_ok());
    }

    #[test]
    fn test_no_op_log_processor() {
        use super::no_op_log_processor::NoOpLogProcessor;

        let mut processor = NoOpLogProcessor::new();

        // Test process_line passes through
        let line = "test output line";
        let result = processor.process_line(line);
        assert_eq!(result, Some(line.to_string()));

        // Test get_full_log
        processor.process_line("line 1");
        processor.process_line("line 2");
        let full_log = processor.get_full_log();
        assert!(full_log.contains("line 1"));
        assert!(full_log.contains("line 2"));

        // Test get_final_result returns None
        assert!(processor.get_final_result().is_none());
    }
}
