use super::Agent;
use crate::context::tsk_env::TskEnv;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

pub mod claude_log_processor;
pub use claude_log_processor::ClaudeLogProcessor;

/// Claude AI agent implementation
pub struct ClaudeAgent {
    tsk_env: Option<Arc<TskEnv>>,
    version_cache: OnceLock<String>,
}

impl ClaudeAgent {
    /// Creates a new ClaudeAgent with the provided TSK environment
    ///
    /// # Arguments
    /// * `tsk_env` - Arc reference to the TskEnv instance containing environment settings
    pub fn with_tsk_env(tsk_env: Arc<TskEnv>) -> Self {
        Self {
            tsk_env: Some(tsk_env),
            version_cache: OnceLock::new(),
        }
    }

    fn get_claude_config_dir(&self) -> PathBuf {
        self.tsk_env
            .as_ref()
            .expect("TskEnv should always be present")
            .claude_config_dir()
            .to_path_buf()
    }
}

impl Default for ClaudeAgent {
    fn default() -> Self {
        Self::with_tsk_env(Arc::new(TskEnv::new().expect("Failed to create TskEnv")))
    }
}

#[async_trait]
impl Agent for ClaudeAgent {
    fn build_command(&self, instruction_path: &str, is_interactive: bool) -> Vec<String> {
        // Get just the filename from the instruction path
        let filename = Path::new(instruction_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("instructions.md");

        if is_interactive {
            let normal_command = format!(
                "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions",
                filename
            );

            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    r#"sleep 0.5; echo '=== Task Instructions ==='; cat /instructions/{}; echo; echo '=== Normal Command ==='; echo '{}'; echo; echo '=== Starting Interactive Claude Code Session ==='; echo 'You can now interact with Claude Code directly.'; echo 'Type "exit" or Ctrl+D to end the session.'; echo; exec /bin/bash"#,
                    filename, normal_command
                ),
            ]
        } else {
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    // Pipe instructions to claude, capture all output (stdout + stderr), and tee to log file
                    // This allows TSK to process output in real-time while preserving a complete log
                    "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions 2>&1 | tee /output/claude-log.txt",
                    filename
                ),
            ]
        }
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        let claude_config_dir = self.get_claude_config_dir();

        vec![
            // Claude config directory
            (
                claude_config_dir.to_string_lossy().to_string(),
                "/home/agent/.claude".to_string(),
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
        task: Option<&crate::task::Task>,
    ) -> Box<dyn super::LogProcessor> {
        let task_name = task.map(|t| t.name.clone());
        Box::new(ClaudeLogProcessor::new(task_name))
    }

    fn name(&self) -> &str {
        "claude"
    }

    async fn validate(&self) -> Result<(), String> {
        // Skip validation in test environments
        if cfg!(test) {
            return Ok(());
        }

        // Check if ~/.claude directory exists
        let claude_config_dir = self.get_claude_config_dir();

        if !claude_config_dir.exists() {
            return Err(format!(
                "Claude configuration directory not found at {}. Please run 'claude login' first.",
                claude_config_dir.display()
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

    fn version(&self) -> String {
        self.version_cache
            .get_or_init(|| {
                // Skip version detection in test environments
                if cfg!(test) {
                    return "test-claude-1.0.0".to_string();
                }

                // Try to get Claude version from CLI
                match Command::new("claude").arg("--version").output() {
                    Ok(output) if output.status.success() => {
                        let version_str = String::from_utf8_lossy(&output.stdout);
                        // Parse version string, typically in format "claude 0.3.0" or similar
                        // Clean up the version string to remove extra whitespace and newlines
                        let cleaned = version_str.trim().to_string();
                        if cleaned.is_empty() {
                            "unknown".to_string()
                        } else {
                            // Replace spaces with hyphens for Docker compatibility
                            cleaned.replace(' ', "-").replace('\n', "")
                        }
                    }
                    Ok(_) => {
                        eprintln!("Warning: Failed to get Claude version (command failed)");
                        "unknown".to_string()
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to run claude --version: {}", e);
                        "unknown".to_string()
                    }
                }
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[test]
    fn test_claude_agent_properties() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env);

        // Test name
        assert_eq!(agent.name(), "claude");

        // Test volumes
        let volumes = agent.volumes();
        assert_eq!(volumes.len(), 1);

        // Should mount .claude directory
        let volume_paths: Vec<&str> = volumes
            .iter()
            .map(|(_, container_path, _)| container_path.as_str())
            .collect();
        assert!(volume_paths.contains(&"/home/agent/.claude"));

        // Test environment variables
        let env = agent.environment();
        assert_eq!(env.len(), 2);

        let env_map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(env_map.get("HOME"), Some(&"/home/agent".to_string()));
        assert_eq!(env_map.get("USER"), Some(&"agent".to_string()));
    }

    #[test]
    fn test_claude_agent_build_command() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env);

        // Test non-interactive mode with full path
        let command = agent.build_command("/tmp/instructions.md", false);
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p --verbose --output-format stream-json"));
        assert!(command[2].contains("tee /output/claude-log.txt"));

        // Test non-interactive mode with complex path
        let command = agent.build_command("/path/to/task/instructions.txt", false);
        assert!(command[2].contains("cat /instructions/instructions.txt"));
        assert!(command[2].contains("tee /output/claude-log.txt"));

        // Test interactive mode
        let command = agent.build_command("/tmp/instructions.md", true);
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");

        // Check that it has a sleep to allow attach to be ready
        assert!(command[2].starts_with("sleep 0.5;"));

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
    async fn test_claude_agent_validate_without_config() {
        // In test mode, validation is skipped so this test just verifies
        // that validate() returns Ok in test environments
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env);
        let result = agent.validate().await;

        // In test mode, validation is skipped
        assert!(result.is_ok());
    }

    #[test]
    fn test_claude_agent_create_log_processor() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env);

        let log_processor = agent.create_log_processor(None);

        // Just verify we can create a log processor
        // The actual log processor functionality is tested elsewhere
        let _ = log_processor.get_full_log();

        // Also test with custom config using AppContext
        use crate::context::AppContext;
        let ctx = AppContext::builder().build();
        let agent_with_config = ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = agent_with_config.create_log_processor(None);
    }

    #[test]
    fn test_claude_agent_version() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env);

        // In test mode, should return test version
        let version = agent.version();
        assert_eq!(version, "test-claude-1.0.0");

        // Test that calling version multiple times returns the same cached value
        let version2 = agent.version();
        assert_eq!(version, version2);
    }
}
