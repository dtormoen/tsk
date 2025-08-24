use async_trait::async_trait;

use crate::agent::{LogProcessor, TaskResult};

/// A simple log processor for the no-op agent that passes through all lines.
///
/// This processor doesn't do any special formatting or parsing since the
/// no-op agent just outputs the instructions file directly.
pub struct NoOpLogProcessor {
    full_log: Vec<String>,
}

impl NoOpLogProcessor {
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
        }
    }
}

#[async_trait]

impl LogProcessor for NoOpLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        // Store the line for the full log
        self.full_log.push(line.to_string());

        // Pass through all lines without modification
        Some(line.to_string())
    }

    fn get_final_result(&self) -> Option<&TaskResult> {
        // No-op doesn't produce a task result
        None
    }

    fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }
}
