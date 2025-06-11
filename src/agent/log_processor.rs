use async_trait::async_trait;
use std::path::Path;

/// Trait for processing agent log output
#[async_trait]
pub trait LogProcessor: Send {
    /// Process a single line of log output
    /// Returns Some(formatted_output) if the line should be displayed, None otherwise
    fn process_line(&mut self, line: &str) -> Option<String>;

    /// Get the full log content
    #[allow(dead_code)]
    fn get_full_log(&self) -> String;

    /// Save the full log to a file
    async fn save_full_log(&self, path: &Path) -> Result<(), String>;

    /// Get the final result of the task execution
    fn get_final_result(&self) -> Option<&crate::log_processor::TaskResult>;
}
