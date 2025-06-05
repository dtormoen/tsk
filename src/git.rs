use crate::context::file_system::FileSystemOperations;
use crate::context::git_operations::GitOperations;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Factory function to get a RepoManager instance
/// Returns a dummy implementation in test mode that panics on use
#[cfg(not(test))]
pub fn get_repo_manager(
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
) -> RepoManager {
    RepoManager::new(file_system, git_operations)
}

#[cfg(test)]
pub fn get_repo_manager(
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
) -> RepoManager {
    RepoManager {
        base_path: PathBuf::from(".tsk/tasks"),
        file_system,
        git_operations,
        repo_root: None,
    }
}

pub struct RepoManager {
    base_path: PathBuf,
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
    repo_root: Option<PathBuf>,
}

impl RepoManager {
    #[cfg(not(test))]
    pub fn new(
        file_system: Arc<dyn FileSystemOperations>,
        git_operations: Arc<dyn GitOperations>,
    ) -> Self {
        Self {
            base_path: PathBuf::from(".tsk/tasks"),
            file_system,
            git_operations,
            repo_root: None,
        }
    }

    #[cfg(test)]
    pub fn with_git_operations(
        file_system: Arc<dyn FileSystemOperations>,
        git_operations: Arc<dyn GitOperations>,
    ) -> Self {
        Self {
            base_path: PathBuf::from(".tsk/tasks"),
            file_system,
            git_operations,
            repo_root: None,
        }
    }

    /// Create a RepoManager with a specific repository root path
    #[cfg(test)]
    pub fn with_repo_root(
        file_system: Arc<dyn FileSystemOperations>,
        git_operations: Arc<dyn GitOperations>,
        repo_root: PathBuf,
    ) -> Self {
        Self {
            base_path: repo_root.join(".tsk/tasks"),
            file_system,
            git_operations,
            repo_root: Some(repo_root),
        }
    }

    /// Copy repository for a task with the name `<task-name>` in `.tsk/tasks/<task-name>/repo-<task-name>`
    /// Returns the path to the copied repository and the branch name
    pub async fn copy_repo(&self, task_name: &str) -> Result<(PathBuf, String), String> {
        // Generate timestamp in YYYY-MM-DD-HHMM format
        let now: DateTime<Local> = Local::now();
        let timestamp = now.format("%Y-%m-%d-%H%M").to_string();

        // Create unique names with timestamp
        let task_dir_name = format!("{}-{}", timestamp, task_name);
        let branch_name = format!("tsk/{}-{}", timestamp, task_name);

        // Create the task directory structure
        let task_dir = self.base_path.join(&task_dir_name);
        let repo_path = task_dir.join(format!("repo-{}", task_name));

        // Create directories if they don't exist
        self.file_system
            .create_dir(&task_dir)
            .await
            .map_err(|e| format!("Failed to create task directory: {}", e))?;

        // Check if we're in a git repository
        if !self.git_operations.is_git_repository().await? {
            return Err("Not in a git repository".to_string());
        }

        // Get the repository root
        let current_dir = match &self.repo_root {
            Some(root) => root.clone(),
            None => std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?,
        };

        // Copy the repository, excluding .tsk directory
        self.copy_directory(&current_dir, &repo_path).await?;

        // Create a new branch in the copied repository
        self.git_operations
            .create_branch(&repo_path, &branch_name)
            .await?;

        println!("Created repository copy at: {}", repo_path.display());
        println!("Branch: {}", branch_name);
        Ok((repo_path, branch_name))
    }

    /// Copy directory recursively, excluding .tsk directory
    #[allow(clippy::only_used_in_recursion)]
    async fn copy_directory(&self, src: &Path, dst: &Path) -> Result<(), String> {
        self.file_system
            .create_dir(dst)
            .await
            .map_err(|e| format!("Failed to create destination directory: {}", e))?;

        let entries = self
            .file_system
            .read_dir(src)
            .await
            .map_err(|e| format!("Failed to read directory: {}", e))?;

        for path in entries {
            let file_name = path
                .file_name()
                .ok_or_else(|| "Invalid file name".to_string())?;

            // Skip .tsk directory
            if file_name == ".tsk" {
                continue;
            }

            let dst_path = dst.join(file_name);

            // Check if it's a directory by trying to read it as one
            if self.file_system.read_dir(&path).await.is_ok() {
                Box::pin(self.copy_directory(&path, &dst_path)).await?;
            } else {
                self.file_system
                    .copy_file(&path, &dst_path)
                    .await
                    .map_err(|e| format!("Failed to copy file {}: {}", path.display(), e))?;
            }
        }

        Ok(())
    }

    /// Commit any uncommitted changes in the repository
    pub async fn commit_changes(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        // Check if there are any changes to commit
        let status_output = self.git_operations.get_status(repo_path).await?;

        if status_output.trim().is_empty() {
            println!("No changes to commit");
            return Ok(());
        }

        // Add all changes
        self.git_operations.add_all(repo_path).await?;

        // Commit changes
        self.git_operations.commit(repo_path, message).await?;

        println!("Committed changes: {}", message);
        Ok(())
    }

    /// Fetch changes from the copied repository back to the main repository
    pub async fn fetch_changes(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        // Get the main repository path
        let main_repo = match &self.repo_root {
            Some(root) => root.clone(),
            None => std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?,
        };

        // Add the copied repository as a remote in the main repository
        let now: DateTime<Local> = Local::now();
        let remote_name = format!("tsk-temp-{}", now.format("%Y-%m-%d-%H%M%S"));

        self.git_operations
            .add_remote(&main_repo, &remote_name, repo_path_str)
            .await?;

        // Fetch the specific branch from the remote
        match self
            .git_operations
            .fetch_branch(&main_repo, &remote_name, branch_name)
            .await
        {
            Ok(_) => {
                // Remove the temporary remote
                self.git_operations
                    .remove_remote(&main_repo, &remote_name)
                    .await?;
            }
            Err(e) => {
                // Remove the temporary remote before returning error
                let _ = self
                    .git_operations
                    .remove_remote(&main_repo, &remote_name)
                    .await;
                return Err(e);
            }
        }

        println!("Fetched changes from copied repository");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::git_operations::tests::MockGitOperations;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_copy_repo_not_in_git_repo() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base_path = temp_dir.path().join(".tsk/tasks");

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(false));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let manager = RepoManager {
            base_path: base_path.clone(),
            file_system: fs,
            git_operations: mock_git_ops,
            repo_root: None,
        };

        let result = manager.copy_repo("test-task").await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in a git repository");
    }

    #[tokio::test]
    async fn test_commit_changes_no_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_get_status_result(Ok("".to_string()));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let manager = RepoManager {
            base_path: PathBuf::from(".tsk/tasks"),
            file_system: fs,
            git_operations: mock_git_ops,
            repo_root: None,
        };

        let result = manager.commit_changes(repo_path, "Test commit").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_commit_changes_with_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_get_status_result(Ok("M file.txt\n".to_string()));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let manager = RepoManager {
            base_path: PathBuf::from(".tsk/tasks"),
            file_system: fs,
            git_operations: mock_git_ops,
            repo_root: None,
        };

        let result = manager.commit_changes(repo_path, "Test commit").await;

        assert!(result.is_ok());
    }
}
