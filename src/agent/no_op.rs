use super::{Agent, LogProcessor};
use async_trait::async_trait;

/// A no-op agent that simply outputs the instructions file.
///
/// This agent is primarily used for debug sessions where we want to show
/// the instructions but not execute any actual AI agent. It uses `cat`
/// to display the instructions file and then exits.
pub struct NoOpAgent;

#[async_trait]
impl Agent for NoOpAgent {
    fn build_command(&self, instructions_path: &str) -> Vec<String> {
        // Simple command to output the instructions and exit
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "echo '=== Task Instructions ==='; cat '{}'; echo; echo '=== End Instructions ==='",
                instructions_path
            ),
        ]
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        // No special volumes needed
        vec![]
    }

    fn environment(&self) -> Vec<(String, String)> {
        // Minimal environment
        vec![]
    }

    fn create_log_processor(&self) -> Box<dyn LogProcessor> {
        Box::new(super::no_op_log_processor::NoOpLogProcessor::new())
    }

    fn name(&self) -> &'static str {
        "no-op"
    }

    async fn validate(&self) -> Result<(), String> {
        // No validation needed for no-op agent
        Ok(())
    }

    async fn warmup(&self) -> Result<(), String> {
        // No warmup needed for no-op agent
        Ok(())
    }
}
