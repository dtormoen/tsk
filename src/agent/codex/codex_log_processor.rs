use async_trait::async_trait;

use crate::agent::{LogProcessor, TaskResult};

/// A simple log processor for the codex agent that passes through all lines.
///
/// This processor doesn't do any special formatting or parsing since
/// Codex already has nice output that doesn't require processing.
pub struct CodexLogProcessor {
    full_log: Vec<String>,
}

impl CodexLogProcessor {
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
        }
    }
}

#[async_trait]
impl LogProcessor for CodexLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        // Store the line for the full log
        self.full_log.push(line.to_string());

        // Pass through all lines without modification
        Some(line.to_string())
    }

    fn get_final_result(&self) -> Option<&TaskResult> {
        // Codex doesn't produce a structured task result
        None
    }

    fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }
}
