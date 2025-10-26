use async_trait::async_trait;

mod claude;
mod codex;
mod log_processor;
mod no_op;
mod no_op_log_processor;
mod provider;
mod task_result;

pub use self::log_processor::LogProcessor;
pub use self::task_result::TaskResult;
pub use claude::ClaudeAgent;
pub use codex::CodexAgent;
pub use no_op::NoOpAgent;
pub use provider::AgentProvider;

/// Trait defining the interface for AI agents that can execute tasks
#[async_trait]
pub trait Agent: Send + Sync {
    /// Returns the command to execute the agent
    ///
    /// # Arguments
    /// * `instruction_path` - Path to the instruction file
    /// * `is_interactive` - Whether to build command for interactive debugging mode
    ///
    /// When `is_interactive` is true, the command should:
    /// 1. Echo the task instructions and normal command that would run non-interactively
    /// 2. Provide an interactive shell or interface for debugging
    fn build_command(&self, instruction_path: &str, is_interactive: bool) -> Vec<String>;

    /// Returns the volumes to mount for this agent
    /// Format: Vec<(host_path, container_path, options)> where options is like ":ro" for read-only
    fn volumes(&self) -> Vec<(String, String, String)>;

    /// Returns environment variables for this agent
    fn environment(&self) -> Vec<(String, String)>;

    /// Creates a log processor for this agent's output
    fn create_log_processor(&self, task: Option<&crate::task::Task>) -> Box<dyn LogProcessor>;

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

    /// Returns the version string for this agent
    ///
    /// This version is used to determine when Docker images need to be rebuilt.
    /// When an agent's version changes, Docker will rebuild the image from the agent
    /// layer onwards, ensuring users always have the latest agent version.
    ///
    /// The default implementation returns "unknown" for backward compatibility.
    /// Agents should override this to return their actual version string.
    fn version(&self) -> String {
        "unknown".to_string()
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
        fn build_command(&self, instruction_path: &str, is_interactive: bool) -> Vec<String> {
            if is_interactive {
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    format!(
                        "sleep 0.5; echo '=== Agent Command ==='; echo 'test {}'; echo '=== Starting Interactive Session ==='; exec /bin/bash",
                        instruction_path
                    ),
                ]
            } else {
                vec!["test".to_string(), instruction_path.to_string()]
            }
        }

        fn volumes(&self) -> Vec<(String, String, String)> {
            vec![("/test".to_string(), "/test".to_string(), ":ro".to_string())]
        }

        fn environment(&self) -> Vec<(String, String)> {
            vec![("TEST_VAR".to_string(), "test_value".to_string())]
        }

        fn create_log_processor(&self, _task: Option<&crate::task::Task>) -> Box<dyn LogProcessor> {
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

        fn version(&self) -> String {
            "test-1.0.0".to_string()
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

        // Test build_command in non-interactive mode
        let command = agent.build_command("/instructions/test.md", false);
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat '/instructions/test.md'"));

        // Test build_command in interactive mode
        let interactive_command = agent.build_command("/instructions/test.md", true);
        assert_eq!(interactive_command.len(), 3);
        assert_eq!(interactive_command[0], "sh");
        assert_eq!(interactive_command[1], "-c");
        assert!(interactive_command[2].starts_with("sleep 0.5;"));
        assert!(interactive_command[2].contains("=== Task Instructions ==="));
        assert!(interactive_command[2].contains("=== Starting Interactive Session ==="));

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

        // Test version (should return "1.0.0" for no-op agent)
        assert_eq!(agent.version(), "1.0.0");
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

    #[test]
    fn test_codex_log_processor() {
        use super::codex::codex_log_processor::CodexLogProcessor;

        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Test that thread.started is filtered out
        let thread_started = r#"{"type":"thread.started","thread_id":"test-123"}"#;
        assert_eq!(processor.process_line(thread_started), None);

        // Test command execution
        let cmd_started = r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"ls","status":"in_progress"}}"#;
        let output = processor.process_line(cmd_started);
        assert!(output.is_some());
        assert!(output.unwrap().contains("Running: ls"));

        // Test get_full_log stores lines
        let full_log = processor.get_full_log();
        assert!(full_log.contains("thread.started"));
        assert!(full_log.contains("command_execution"));

        // Test get_final_result returns None before turn.completed
        assert!(processor.get_final_result().is_none());

        // Test turn.completed creates final result
        let turn_completed =
            r#"{"type":"turn.completed","usage":{"input_tokens":1000,"output_tokens":500}}"#;
        processor.process_line(turn_completed);
        let result = processor.get_final_result();
        assert!(result.is_some());
        assert!(result.unwrap().success);
    }
}
