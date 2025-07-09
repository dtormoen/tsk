use async_trait::async_trait;

/// Trait for processing agent log output
#[async_trait]
pub trait LogProcessor: Send {
    /// Process a single line of log output
    /// Returns Some(formatted_output) if the line should be displayed, None otherwise
    fn process_line(&mut self, line: &str) -> Option<String>;

    /// Get the full log content
    #[allow(dead_code)]
    fn get_full_log(&self) -> String;

    /// Get the final result of the task execution
    fn get_final_result(&self) -> Option<&super::TaskResult>;
}
