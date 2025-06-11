use super::{Agent, LogProcessor};
use crate::context::file_system::FileSystemOperations;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

/// Claude Code AI agent implementation
pub struct ClaudeCodeAgent {
    docker_image: String,
}

impl ClaudeCodeAgent {
    pub fn new() -> Self {
        Self {
            docker_image: "tsk/base".to_string(),
        }
    }
}

impl Default for ClaudeCodeAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    fn docker_image(&self) -> &str {
        &self.docker_image
    }

    fn build_command(&self, instruction_path: &str) -> Vec<String> {
        // Get just the filename from the instruction path
        let filename = Path::new(instruction_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("instructions.md");

        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions",
                filename
            ),
        ]
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        // Get the home directory path for mounting ~/.claude and ~/.claude.json
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/home/agent".to_string());

        vec![
            // Claude config directory
            (
                format!("{}/.claude", home_dir),
                "/home/agent/.claude".to_string(),
                "".to_string(),
            ),
            // Claude config file
            (
                format!("{}/.claude.json", home_dir),
                "/home/agent/.claude.json".to_string(),
                "".to_string(),
            ),
        ]
    }

    fn environment(&self) -> Vec<(String, String)> {
        vec![
            ("HOME".to_string(), "/home/agent".to_string()),
            ("USER".to_string(), "agent".to_string()),
        ]
    }

    fn create_log_processor(
        &self,
        file_system: Arc<dyn FileSystemOperations>,
    ) -> Box<dyn LogProcessor> {
        Box::new(ClaudeCodeLogProcessor::new(file_system))
    }

    fn name(&self) -> &str {
        "claude-code"
    }

    async fn validate(&self) -> Result<(), String> {
        // Check if ~/.claude.json exists
        let home_dir = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
        let claude_config = Path::new(&home_dir).join(".claude.json");

        if !claude_config.exists() {
            return Err(format!(
                "Claude configuration not found at {}. Please run 'claude login' first.",
                claude_config.display()
            ));
        }

        Ok(())
    }
}

/// Claude Code specific log processor
struct ClaudeCodeLogProcessor {
    inner: crate::log_processor::LogProcessor,
}

impl ClaudeCodeLogProcessor {
    fn new(file_system: Arc<dyn FileSystemOperations>) -> Self {
        Self {
            inner: crate::log_processor::LogProcessor::with_file_system(file_system),
        }
    }
}

#[async_trait]
impl LogProcessor for ClaudeCodeLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        self.inner.process_line(line)
    }

    fn get_full_log(&self) -> String {
        self.inner.get_full_log()
    }

    async fn save_full_log(&self, path: &Path) -> Result<(), String> {
        self.inner.save_full_log(path).await
    }

    fn get_final_result(&self) -> Option<&crate::log_processor::TaskResult> {
        self.inner.get_final_result()
    }
}
