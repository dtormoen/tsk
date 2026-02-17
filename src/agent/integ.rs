use super::{Agent, LogProcessor};
use async_trait::async_trait;

/// An integration test agent that runs tsk-integ-test.sh from the project workspace.
///
/// This agent is used for integration testing to verify that TSK stack layers
/// work correctly with real projects. It looks for a `tsk-integ-test.sh` script
/// in the workspace root and executes it.
pub struct IntegAgent;

#[async_trait]
impl Agent for IntegAgent {
    fn build_command(&self, _instructions_path: &str, is_interactive: bool) -> Vec<String> {
        if is_interactive {
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo '=== Integration Test Agent ==='; echo 'Running tsk-integ-test.sh...'; cd /workspace/* && if [ -f tsk-integ-test.sh ]; then bash tsk-integ-test.sh; fi; echo '=== Starting Interactive Session ==='; exec /bin/bash".to_string(),
            ]
        } else {
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "cd /workspace/* && if [ -f tsk-integ-test.sh ]; then bash tsk-integ-test.sh; else echo 'ERROR: No tsk-integ-test.sh found'; exit 1; fi".to_string(),
            ]
        }
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        vec![]
    }

    fn environment(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn create_log_processor(&self, _task: Option<&crate::task::Task>) -> Box<dyn LogProcessor> {
        Box::new(super::no_op_log_processor::NoOpLogProcessor::new())
    }

    fn name(&self) -> &'static str {
        "integ"
    }

    async fn validate(&self) -> Result<(), String> {
        Ok(())
    }

    async fn warmup(&self) -> Result<(), String> {
        Ok(())
    }

    fn version(&self) -> String {
        "1.0.0".to_string()
    }
}
