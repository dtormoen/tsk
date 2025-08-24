use super::Agent;
use crate::context::tsk_config::TskConfig;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

mod claude_code_log_processor;
use claude_code_log_processor::ClaudeCodeLogProcessor;

/// Claude Code AI agent implementation
pub struct ClaudeCodeAgent {
    tsk_config: Option<Arc<TskConfig>>,
}

impl ClaudeCodeAgent {
    /// Creates a new ClaudeCodeAgent with the provided TSK configuration
    ///
    /// # Arguments
    /// * `tsk_config` - Arc reference to the TskConfig instance containing environment settings
    pub fn with_tsk_config(tsk_config: Arc<TskConfig>) -> Self {
        Self {
            tsk_config: Some(tsk_config),
        }
    }

    fn get_claude_config_dir(&self) -> PathBuf {
        self.tsk_config
            .as_ref()
            .expect("TskConfig should always be present")
            .claude_config_dir()
            .to_path_buf()
    }
}

impl Default for ClaudeCodeAgent {
    fn default() -> Self {
        Self::with_tsk_config(Arc::new(
            TskConfig::new(None).expect("Failed to create TskConfig"),
        ))
    }
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
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

    fn build_interactive_command(&self, instruction_path: &str) -> Vec<String> {
        // Get just the filename from the instruction path
        let filename = Path::new(instruction_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("instructions.md");

        let normal_command = format!(
            "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions",
            filename
        );

        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                r#"echo '=== Task Instructions ==='; cat /instructions/{}; echo; echo '=== Normal Command ==='; echo '{}'; echo; echo '=== Starting Interactive Claude Code Session ==='; echo 'You can now interact with Claude Code directly.'; echo 'Type "exit" or Ctrl+D to end the session.'; echo; exec /bin/bash"#,
                filename, normal_command
            ),
        ]
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        let claude_config_dir = self.get_claude_config_dir();
        let claude_json_path = claude_config_dir.with_extension("json");

        vec![
            // Claude config directory
            (
                claude_config_dir.to_string_lossy().to_string(),
                "/home/agent/.claude".to_string(),
                "".to_string(),
            ),
            // Claude config file
            (
                claude_json_path.to_string_lossy().to_string(),
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

    fn create_log_processor(&self) -> Box<dyn super::LogProcessor> {
        Box::new(ClaudeCodeLogProcessor::new())
    }

    fn name(&self) -> &str {
        "claude-code"
    }

    async fn validate(&self) -> Result<(), String> {
        // Skip validation in test environments
        if cfg!(test) {
            return Ok(());
        }

        // Check if ~/.claude.json exists
        let claude_config = self.get_claude_config_dir().with_extension("json");

        if !claude_config.exists() {
            return Err(format!(
                "Claude configuration not found at {}. Please run 'claude login' first.",
                claude_config.display()
            ));
        }

        Ok(())
    }

    async fn warmup(&self) -> Result<(), String> {
        // Skip warmup in test environments
        if cfg!(test) {
            return Ok(());
        }

        println!("Running Claude Code warmup steps...");

        // Step 1: Force Claude CLI to refresh token
        let output = Command::new("claude")
            .args(["-p", "--model", "sonnet", "say hi and nothing else"])
            .output()
            .map_err(|e| format!("Failed to run Claude CLI: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "Claude CLI failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Step 2: Export OAuth token on macOS only
        if std::env::consts::OS == "macos" {
            let user = std::env::var("USER").map_err(|_| "USER environment variable not set")?;

            let output = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "security find-generic-password -a {user} -w -s 'Claude Code-credentials' > ~/.claude/.credentials.json"
                ))
                .output()
                .map_err(|e| format!("Failed to export OAuth token: {e}"))?;

            if !output.status.success() {
                // This might fail if the keychain item doesn't exist yet
                eprintln!("Warning: Could not export OAuth token from keychain");
            }
        }

        println!("Claude Code warmup completed successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_code_agent_properties() {
        let tsk_config = Arc::new(TskConfig::new(None).unwrap());
        let agent = ClaudeCodeAgent::with_tsk_config(tsk_config);

        // Test name
        assert_eq!(agent.name(), "claude-code");

        // Test volumes
        let volumes = agent.volumes();
        assert_eq!(volumes.len(), 2);

        // Should mount .claude directory and .claude.json file
        let volume_paths: Vec<&str> = volumes
            .iter()
            .map(|(_, container_path, _)| container_path.as_str())
            .collect();
        assert!(volume_paths.contains(&"/home/agent/.claude"));
        assert!(volume_paths.contains(&"/home/agent/.claude.json"));

        // Test environment variables
        let env = agent.environment();
        assert_eq!(env.len(), 2);

        let env_map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(env_map.get("HOME"), Some(&"/home/agent".to_string()));
        assert_eq!(env_map.get("USER"), Some(&"agent".to_string()));
    }

    #[test]
    fn test_claude_code_agent_build_command() {
        let tsk_config = Arc::new(TskConfig::new(None).unwrap());
        let agent = ClaudeCodeAgent::with_tsk_config(tsk_config);

        // Test with full path
        let command = agent.build_command("/tmp/instructions.md");
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p --verbose --output-format stream-json"));

        // Test with complex path
        let command = agent.build_command("/path/to/task/instructions.txt");
        assert!(command[2].contains("cat /instructions/instructions.txt"));
    }

    #[test]
    fn test_claude_code_agent_build_interactive_command() {
        let tsk_config = Arc::new(TskConfig::new(None).unwrap());
        let agent = ClaudeCodeAgent::with_tsk_config(tsk_config);

        // Test interactive command
        let command = agent.build_interactive_command("/tmp/instructions.md");
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");

        // Check that it shows the instructions
        assert!(command[2].contains("=== Task Instructions ==="));
        assert!(command[2].contains("cat /instructions/instructions.md"));

        // Check that it shows the normal command
        assert!(command[2].contains("=== Normal Command ==="));
        assert!(command[2].contains("claude -p --verbose --output-format stream-json"));

        // Check that it starts an interactive session
        assert!(command[2].contains("=== Starting Interactive Claude Code Session ==="));
        assert!(command[2].contains("exec /bin/bash"));
    }

    #[tokio::test]
    async fn test_claude_code_agent_validate_without_config() {
        // In test mode, validation is skipped so this test just verifies
        // that validate() returns Ok in test environments
        let temp_dir = tempfile::tempdir().unwrap();
        let xdg_config = crate::context::tsk_config::XdgConfig::builder()
            .with_claude_config_dir(temp_dir.path().join(".claude"))
            .build();
        let tsk_config = Arc::new(TskConfig::new(Some(xdg_config)).unwrap());
        let agent = ClaudeCodeAgent::with_tsk_config(tsk_config);
        let result = agent.validate().await;

        // In test mode, validation is skipped
        assert!(result.is_ok());
    }

    #[test]
    fn test_claude_code_agent_create_log_processor() {
        let tsk_config = Arc::new(TskConfig::new(None).unwrap());
        let agent = ClaudeCodeAgent::with_tsk_config(tsk_config);

        let log_processor = agent.create_log_processor();

        // Just verify we can create a log processor
        // The actual log processor functionality is tested elsewhere
        let _ = log_processor.get_full_log();

        // Also test with custom config
        let temp_dir = tempfile::tempdir().unwrap();
        let xdg_config = crate::context::tsk_config::XdgConfig::builder()
            .with_claude_config_dir(temp_dir.path().join(".claude"))
            .build();
        let tsk_config = Arc::new(TskConfig::new(Some(xdg_config)).unwrap());
        let agent_with_config = ClaudeCodeAgent::with_tsk_config(tsk_config);
        let _ = agent_with_config.create_log_processor();
    }
}
