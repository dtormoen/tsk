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
    fn build_command(&self, instructions_path: &str, is_interactive: bool) -> Vec<String> {
        if is_interactive {
            // Interactive mode: show instructions, then provide bash shell
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "sleep 0.5; echo '=== Task Instructions ==='; cat '{}'; echo; echo '=== Starting Interactive Session ==='; exec /bin/bash",
                    instructions_path
                ),
            ]
        } else {
            // Non-interactive mode: just output the instructions
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "echo '=== Task Instructions ==='; cat '{}'; echo; echo '=== End Instructions ==='",
                    instructions_path
                ),
            ]
        }
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        // No special volumes needed
        vec![]
    }

    fn environment(&self) -> Vec<(String, String)> {
        // Minimal environment
        vec![]
    }

    fn create_log_processor(&self, _task: Option<&crate::task::Task>) -> Box<dyn LogProcessor> {
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

    fn version(&self) -> String {
        // Static version for no-op agent since it doesn't change
        "1.0.0".to_string()
    }
}
