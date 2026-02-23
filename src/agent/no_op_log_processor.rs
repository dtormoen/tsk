use async_trait::async_trait;

use crate::agent::log_line::LogLine;
use crate::agent::{LogProcessor, TaskResult};

/// A simple log processor for the no-op agent that passes through all lines.
///
/// This processor doesn't do any special formatting or parsing since the
/// no-op agent just outputs the instructions file directly.
pub struct NoOpLogProcessor;

impl NoOpLogProcessor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LogProcessor for NoOpLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<LogLine> {
        // Pass through all lines as Info-level messages without modification
        Some(LogLine::message(vec![], None, line.to_string()))
    }

    fn get_final_result(&self) -> Option<&TaskResult> {
        // No-op doesn't produce a task result
        None
    }
}
