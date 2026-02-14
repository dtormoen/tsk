use super::Agent;
use crate::context::tsk_env::TskEnv;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

pub mod claude_log_processor;
pub use claude_log_processor::ClaudeLogProcessor;

const NOT_LOGGED_IN_ERROR: &str = "Claude CLI is not logged in. Please run 'claude login' first.";

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

    /// Checks login status via `claude auth status --json`.
    /// Returns `Some(true)` if logged in, `Some(false)` if not logged in,
    /// or `None` if the command is unavailable (older CLI versions).
    fn check_auth_status() -> Option<bool> {
        let output = Command::new("claude")
            .args(["auth", "status", "--json"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let status: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
        Some(status.get("loggedIn") == Some(&serde_json::Value::Bool(true)))
    }
}

impl Default for ClaudeAgent {
    fn default() -> Self {
        Self::with_tsk_env(Arc::new(TskEnv::new().expect("Failed to create TskEnv")))
    }
}

/// Creates a tar archive containing a single file
///
/// # Arguments
/// * `filename` - The name of the file to store in the archive
/// * `contents` - The file contents as bytes
///
/// # Returns
/// The tar archive data as a byte vector
fn create_tar_with_file(filename: &str, contents: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());

    let mut header = tar::Header::new_gnu();
    header.set_path(filename).expect("Invalid filename");
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    // UID/GID 1000 matches the 'agent' user in TSK Docker images
    header.set_uid(1000);
    header.set_gid(1000);
    header.set_mtime(0);
    header.set_cksum();

    builder
        .append(&header, contents)
        .expect("Failed to append file to tar");

    builder.into_inner().expect("Failed to finish tar archive")
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

        // Try `claude auth status` first (available in newer CLI versions)
        if let Some(logged_in) = Self::check_auth_status() {
            return if logged_in {
                Ok(())
            } else {
                Err(NOT_LOGGED_IN_ERROR.to_string())
            };
        }

        // Fall back to directory check for older CLI versions
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

        // Step 1: Check auth status (prefer `auth status`, fall back to prompt)
        match Self::check_auth_status() {
            Some(true) => {}
            Some(false) => {
                return Err(NOT_LOGGED_IN_ERROR.to_string());
            }
            None => {
                // Fall back to prompting Claude for older CLI versions
                println!("Falling back to Claude Code warmup...");
                let output = Command::new("claude")
                    .args([
                        "-p",
                        "--no-session-persistence",
                        "--tools",
                        "",
                        "--model",
                        "sonnet",
                        "say hi and nothing else",
                    ])
                    .output()
                    .map_err(|e| format!("Failed to run Claude CLI: {e}"))?;

                if !output.status.success() {
                    return Err(format!(
                        "Claude CLI failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
            }
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

    fn files_to_copy(&self) -> Vec<(Vec<u8>, String)> {
        // Get the home directory (parent of .claude config dir)
        let claude_config_dir = self.get_claude_config_dir();
        let home_dir = match claude_config_dir.parent() {
            Some(dir) => dir,
            None => return vec![],
        };

        let claude_json_path = home_dir.join(".claude.json");

        // Read the file if it exists
        if claude_json_path.exists() {
            match std::fs::read(&claude_json_path) {
                Ok(contents) => {
                    let tar_data = create_tar_with_file(".claude.json", &contents);
                    vec![(tar_data, "/home/agent".to_string())]
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to read {}: {}",
                        claude_json_path.display(),
                        e
                    );
                    vec![]
                }
            }
        } else {
            vec![]
        }
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

        // Just verify we can create a log processor
        // The actual log processor functionality is tested elsewhere
        let _log_processor = agent.create_log_processor(None);

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

    #[test]
    fn test_create_tar_with_file() {
        use std::io::Read;
        use tar::Archive;

        let contents = b"test file contents";
        let tar_data = super::create_tar_with_file("test.txt", contents);

        // Verify it's a valid tar archive
        let mut archive = Archive::new(tar_data.as_slice());
        let entries: Vec<_> = archive.entries().unwrap().collect();
        assert_eq!(entries.len(), 1);

        // Re-create archive to read the entry
        let mut archive = Archive::new(tar_data.as_slice());
        let mut entry = archive.entries().unwrap().next().unwrap().unwrap();

        // Verify filename
        assert_eq!(entry.path().unwrap().to_str().unwrap(), "test.txt");

        // Verify contents
        let mut extracted_contents = Vec::new();
        entry.read_to_end(&mut extracted_contents).unwrap();
        assert_eq!(extracted_contents, contents);
    }

    #[test]
    fn test_create_tar_with_file_empty_contents() {
        use tar::Archive;

        let tar_data = super::create_tar_with_file("empty.txt", b"");

        let mut archive = Archive::new(tar_data.as_slice());
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.size(), 0);
    }

    #[test]
    fn test_files_to_copy_with_existing_file() {
        use std::fs;

        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env.clone());

        // Get the home directory where we'll create the test file
        let home_dir = tsk_env.claude_config_dir().parent().unwrap();
        let claude_json_path = home_dir.join(".claude.json");

        // Create test file
        let test_content = r#"{"test": "data"}"#;
        fs::write(&claude_json_path, test_content).unwrap();

        // Test files_to_copy
        let files = agent.files_to_copy();
        assert_eq!(files.len(), 1);

        let (tar_data, dest_path) = &files[0];
        assert_eq!(dest_path, "/home/agent");

        // Verify tar contains the correct file
        use std::io::Read;
        use tar::Archive;

        let mut archive = Archive::new(tar_data.as_slice());
        let mut entry = archive.entries().unwrap().next().unwrap().unwrap();

        assert_eq!(entry.path().unwrap().to_str().unwrap(), ".claude.json");

        let mut extracted_contents = String::new();
        entry.read_to_string(&mut extracted_contents).unwrap();
        assert_eq!(extracted_contents, test_content);
    }

    #[test]
    fn test_files_to_copy_without_file() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();
        let agent = ClaudeAgent::with_tsk_env(tsk_env.clone());

        // Ensure the file doesn't exist
        let home_dir = tsk_env.claude_config_dir().parent().unwrap();
        let claude_json_path = home_dir.join(".claude.json");
        if claude_json_path.exists() {
            std::fs::remove_file(&claude_json_path).unwrap();
        }

        // Test files_to_copy returns empty when file doesn't exist
        let files = agent.files_to_copy();
        assert!(files.is_empty());
    }
}
