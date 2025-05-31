use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// Trait for executing commands - allows mocking in tests
pub trait CommandExecutor {
    fn execute(&self, program: &str, args: &[&str]) -> Result<Output, String>;
}

/// Default implementation that actually runs commands
pub struct SystemCommandExecutor;

impl CommandExecutor for SystemCommandExecutor {
    fn execute(&self, program: &str, args: &[&str]) -> Result<Output, String> {
        Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))
    }
}

pub struct WorktreeManager {
    base_path: PathBuf,
    command_executor: Box<dyn CommandExecutor>,
}

impl WorktreeManager {
    pub fn new() -> Self {
        Self {
            base_path: PathBuf::from(".tsk/tasks"),
            command_executor: Box::new(SystemCommandExecutor),
        }
    }

    /// Create a worktree for a task with the name `<task-name>` in `.tsk/tasks/<task-name>/worktree-<task-name>`
    pub fn create_worktree(&self, task_name: &str) -> Result<PathBuf, String> {
        // Generate timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("Failed to get timestamp: {}", e))?
            .as_secs();

        // Create unique names with timestamp
        let task_dir_name = format!("{}-{}", timestamp, task_name);
        let branch_name = format!("tsk/{}-{}", timestamp, task_name);

        // Create the task directory structure
        let task_dir = self.base_path.join(&task_dir_name);
        let worktree_path = task_dir.join(format!("worktree-{}", task_name));

        // Create directories if they don't exist
        fs::create_dir_all(&task_dir)
            .map_err(|e| format!("Failed to create task directory: {}", e))?;

        // Check if we're in a git repository
        let git_check = self
            .command_executor
            .execute("git", &["rev-parse", "--git-dir"])?;

        if !git_check.status.success() {
            return Err("Not in a git repository".to_string());
        }

        // Create the worktree with a new branch
        let worktree_cmd = self.command_executor.execute(
            "git",
            &[
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_path.to_str().unwrap(),
            ],
        )?;

        if !worktree_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&worktree_cmd.stderr);
            return Err(format!("Failed to create worktree: {}", stderr));
        }

        println!("Created worktree at: {}", worktree_path.display());
        println!("Branch: {}", branch_name);
        Ok(worktree_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};
    use tempfile::TempDir;

    /// Mock command executor for testing
    struct MockCommandExecutor {
        responses: Vec<(String, Vec<String>, Result<Output, String>)>,
    }

    impl MockCommandExecutor {
        fn new() -> Self {
            Self { responses: vec![] }
        }

        fn add_response(
            &mut self,
            program: &str,
            args: Vec<&str>,
            success: bool,
            stdout: &str,
            stderr: &str,
        ) {
            let output = Output {
                status: ExitStatus::from_raw(if success { 0 } else { 1 }),
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            };
            self.responses.push((
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
                Ok(output),
            ));
        }

        fn add_error(&mut self, program: &str, args: Vec<&str>, error: &str) {
            self.responses.push((
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
                Err(error.to_string()),
            ));
        }
    }

    impl CommandExecutor for MockCommandExecutor {
        fn execute(&self, program: &str, args: &[&str]) -> Result<Output, String> {
            // Special handling for commands with dynamic paths
            if program == "git" && args.len() >= 2 {
                if args[0] == "worktree" && args[1] == "add" {
                    // For worktree add commands, check if it matches our pattern
                    for (cmd_program, cmd_args, response) in &self.responses {
                        if cmd_program == "git"
                            && cmd_args.len() >= 2
                            && cmd_args[0] == "worktree"
                            && cmd_args[1] == "add"
                        {
                            return response.clone();
                        }
                    }
                }
            }

            // Exact match for other commands
            for (cmd_program, cmd_args, response) in &self.responses {
                if cmd_program == program && cmd_args == args {
                    return response.clone();
                }
            }
            Err(format!(
                "No mock response for command: {} {:?}",
                program, args
            ))
        }
    }

    #[test]
    fn test_create_worktree_success() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base_path = temp_dir.path().join(".tsk/tasks");

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();

        // Mock git rev-parse (checking if in git repo)
        mock_executor.add_response("git", vec!["rev-parse", "--git-dir"], true, ".git", "");

        // Mock git worktree add
        mock_executor.add_response(
            "git",
            vec![
                "worktree",
                "add",
                "-b",
                "tsk/test-task",
                "test-worktree-path",
            ],
            true,
            "",
            "",
        );

        let manager = WorktreeManager {
            base_path: base_path.clone(),
            command_executor: Box::new(mock_executor),
        };

        // Note: We need to mock the exact path that will be generated
        // This is a limitation of our current design - we might want to refactor
        // to make timestamp generation mockable as well
        let result = manager.create_worktree("test-task");

        // Since we can't predict the exact timestamp, we just verify it succeeded
        assert!(result.is_ok(), "Should create worktree successfully");
    }

    #[test]
    fn test_create_worktree_not_in_git_repo() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base_path = temp_dir.path().join(".tsk/tasks");

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();

        // Mock git rev-parse failure (not in git repo)
        mock_executor.add_response(
            "git",
            vec!["rev-parse", "--git-dir"],
            false,
            "",
            "fatal: not a git repository",
        );

        let manager = WorktreeManager {
            base_path: base_path.clone(),
            command_executor: Box::new(mock_executor),
        };

        let result = manager.create_worktree("test-task");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in a git repository");
    }

    #[test]
    fn test_create_worktree_command_fails() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base_path = temp_dir.path().join(".tsk/tasks");

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();

        // Mock git rev-parse success
        mock_executor.add_response("git", vec!["rev-parse", "--git-dir"], true, ".git", "");

        // Mock git worktree add failure
        mock_executor.add_error(
            "git",
            vec![
                "worktree",
                "add",
                "-b",
                "tsk/test-task",
                "test-worktree-path",
            ],
            "Command failed",
        );

        let manager = WorktreeManager {
            base_path: base_path.clone(),
            command_executor: Box::new(mock_executor),
        };

        let result = manager.create_worktree("test-task");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Command failed"));
    }

    #[test]
    fn test_directory_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base_path = temp_dir.path().join(".tsk/tasks");

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();
        mock_executor.add_response("git", vec!["rev-parse", "--git-dir"], true, ".git", "");

        // We'll make the git worktree command fail so we can check directory was created
        mock_executor.add_response(
            "git",
            vec![
                "worktree",
                "add",
                "-b",
                "tsk/test-task",
                "test-worktree-path",
            ],
            false,
            "",
            "Some error",
        );

        let manager = WorktreeManager {
            base_path: base_path.clone(),
            command_executor: Box::new(mock_executor),
        };

        let _ = manager.create_worktree("test-task");

        // Verify that .tsk/tasks directory structure was created
        assert!(base_path.exists(), "Base path should be created");
    }
}
