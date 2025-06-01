use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// Factory function to get a RepoManager instance
/// Returns a dummy implementation in test mode that panics on use
#[cfg(not(test))]
pub fn get_repo_manager() -> RepoManager {
    RepoManager::new()
}

#[cfg(test)]
pub fn get_repo_manager() -> RepoManager {
    struct PanicCommandExecutor;

    impl CommandExecutor for PanicCommandExecutor {
        fn execute(&self, _program: &str, _args: &[&str]) -> Result<Output, String> {
            panic!("Git operations are not allowed in tests! Please mock RepoManager properly using RepoManager::with_executor()")
        }
    }

    RepoManager {
        base_path: PathBuf::from(".tsk/tasks"),
        command_executor: Box::new(PanicCommandExecutor),
    }
}

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

pub struct RepoManager {
    base_path: PathBuf,
    command_executor: Box<dyn CommandExecutor>,
}

impl RepoManager {
    pub fn new() -> Self {
        Self {
            base_path: PathBuf::from(".tsk/tasks"),
            command_executor: Box::new(SystemCommandExecutor),
        }
    }

    #[cfg(test)]
    pub fn with_executor(command_executor: Box<dyn CommandExecutor>) -> Self {
        Self {
            base_path: PathBuf::from(".tsk/tasks"),
            command_executor,
        }
    }

    /// Copy repository for a task with the name `<task-name>` in `.tsk/tasks/<task-name>/repo-<task-name>`
    /// Returns the path to the copied repository and the branch name
    pub fn copy_repo(&self, task_name: &str) -> Result<(PathBuf, String), String> {
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
        let repo_path = task_dir.join(format!("repo-{}", task_name));

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

        // Get the current directory (repository root)
        let current_dir = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;

        // Copy the repository, excluding .tsk directory
        self.copy_directory(&current_dir, &repo_path)?;

        // Change to the copied repository directory
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        // Create a new branch in the copied repository
        let checkout_cmd = self.command_executor.execute(
            "git",
            &["-C", repo_path_str, "checkout", "-b", &branch_name],
        )?;

        if !checkout_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_cmd.stderr);
            return Err(format!("Failed to create branch: {}", stderr));
        }

        println!("Created repository copy at: {}", repo_path.display());
        println!("Branch: {}", branch_name);
        Ok((repo_path, branch_name))
    }

    /// Copy directory recursively, excluding .tsk directory
    #[allow(clippy::only_used_in_recursion)]
    fn copy_directory(&self, src: &Path, dst: &Path) -> Result<(), String> {
        fs::create_dir_all(dst)
            .map_err(|e| format!("Failed to create destination directory: {}", e))?;

        for entry in fs::read_dir(src).map_err(|e| format!("Failed to read directory: {}", e))? {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            let file_name = path
                .file_name()
                .ok_or_else(|| "Invalid file name".to_string())?;

            // Skip .tsk directory
            if file_name == ".tsk" {
                continue;
            }

            let dst_path = dst.join(file_name);

            if path.is_dir() {
                self.copy_directory(&path, &dst_path)?;
            } else {
                fs::copy(&path, &dst_path)
                    .map_err(|e| format!("Failed to copy file {}: {}", path.display(), e))?;
            }
        }

        Ok(())
    }

    /// Commit any uncommitted changes in the repository
    pub fn commit_changes(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        // Check if there are any changes to commit
        let status_cmd = self
            .command_executor
            .execute("git", &["-C", repo_path_str, "status", "--porcelain"])?;

        if !status_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&status_cmd.stderr);
            return Err(format!("Failed to check git status: {}", stderr));
        }

        let status_output = String::from_utf8_lossy(&status_cmd.stdout);
        if status_output.trim().is_empty() {
            println!("No changes to commit");
            return Ok(());
        }

        // Add all changes
        let add_cmd = self
            .command_executor
            .execute("git", &["-C", repo_path_str, "add", "-A"])?;

        if !add_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&add_cmd.stderr);
            return Err(format!("Failed to add changes: {}", stderr));
        }

        // Commit changes
        let commit_cmd = self
            .command_executor
            .execute("git", &["-C", repo_path_str, "commit", "-m", message])?;

        if !commit_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&commit_cmd.stderr);
            return Err(format!("Failed to commit changes: {}", stderr));
        }

        println!("Committed changes: {}", message);
        Ok(())
    }

    /// Fetch changes from the copied repository back to the main repository
    pub fn fetch_changes(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        // Get the current directory (main repository)
        let main_repo = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;
        let main_repo_str = main_repo
            .to_str()
            .ok_or_else(|| "Invalid main repo path".to_string())?;

        // Add the copied repository as a remote in the main repository
        let remote_name = format!(
            "tsk-temp-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        let add_remote_cmd = self.command_executor.execute(
            "git",
            &[
                "-C",
                main_repo_str,
                "remote",
                "add",
                &remote_name,
                repo_path_str,
            ],
        )?;

        if !add_remote_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&add_remote_cmd.stderr);
            // If remote already exists, that's okay
            if !stderr.contains("already exists") {
                return Err(format!("Failed to add remote: {}", stderr));
            }
        }

        // Fetch the specific branch from the remote
        let fetch_cmd = self.command_executor.execute(
            "git",
            &[
                "-C",
                main_repo_str,
                "fetch",
                &remote_name,
                &format!("{}:{}", branch_name, branch_name),
            ],
        )?;

        if !fetch_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&fetch_cmd.stderr);
            // Remove the temporary remote before returning error
            let _ = self.command_executor.execute(
                "git",
                &["-C", main_repo_str, "remote", "remove", &remote_name],
            );
            return Err(format!("Failed to fetch changes: {}", stderr));
        }

        // Remove the temporary remote
        let remove_remote_cmd = self.command_executor.execute(
            "git",
            &["-C", main_repo_str, "remote", "remove", &remote_name],
        )?;

        if !remove_remote_cmd.status.success() {
            let stderr = String::from_utf8_lossy(&remove_remote_cmd.stderr);
            return Err(format!("Failed to remove temporary remote: {}", stderr));
        }

        println!("Fetched changes from copied repository");
        Ok(())
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
            // Exact match for commands
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
    fn test_copy_repo_not_in_git_repo() {
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

        let manager = RepoManager {
            base_path: base_path.clone(),
            command_executor: Box::new(mock_executor),
        };

        let result = manager.copy_repo("test-task");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in a git repository");
    }

    #[test]
    fn test_commit_changes_no_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();

        // Mock git status - no changes
        mock_executor.add_response(
            "git",
            vec!["-C", repo_path.to_str().unwrap(), "status", "--porcelain"],
            true,
            "",
            "",
        );

        let manager = RepoManager {
            base_path: PathBuf::from(".tsk/tasks"),
            command_executor: Box::new(mock_executor),
        };

        let result = manager.commit_changes(repo_path, "Test commit");

        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_changes_with_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock executor
        let mut mock_executor = MockCommandExecutor::new();

        // Mock git status - has changes
        mock_executor.add_response(
            "git",
            vec!["-C", repo_path.to_str().unwrap(), "status", "--porcelain"],
            true,
            "M file.txt\n",
            "",
        );

        // Mock git add
        mock_executor.add_response(
            "git",
            vec!["-C", repo_path.to_str().unwrap(), "add", "-A"],
            true,
            "",
            "",
        );

        // Mock git commit
        mock_executor.add_response(
            "git",
            vec![
                "-C",
                repo_path.to_str().unwrap(),
                "commit",
                "-m",
                "Test commit",
            ],
            true,
            "",
            "",
        );

        let manager = RepoManager {
            base_path: PathBuf::from(".tsk/tasks"),
            command_executor: Box::new(mock_executor),
        };

        let result = manager.commit_changes(repo_path, "Test commit");

        assert!(result.is_ok());
    }
}
