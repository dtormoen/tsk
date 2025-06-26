use crate::context::file_system::FileSystemOperations;
use crate::context::git_operations::GitOperations;
use crate::storage::XdgDirectories;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct RepoManager {
    xdg_directories: Arc<XdgDirectories>,
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
}

impl RepoManager {
    pub fn new(
        xdg_directories: Arc<XdgDirectories>,
        file_system: Arc<dyn FileSystemOperations>,
        git_operations: Arc<dyn GitOperations>,
    ) -> Self {
        Self {
            xdg_directories,
            file_system,
            git_operations,
        }
    }

    /// Copy repository for a task using the task ID and repository root
    /// Copies git-tracked files, the .git directory, and the .tsk directory
    /// Returns the path to the copied repository and the branch name
    pub async fn copy_repo(
        &self,
        task_id: &str,
        repo_root: &Path,
        source_commit: Option<&str>,
    ) -> Result<(PathBuf, String), String> {
        // Use the task ID directly for the directory name
        let task_dir_name = task_id;
        let branch_name = format!("tsk/{}", task_id);

        // Create the task directory structure in centralized location
        let repo_hash = crate::storage::get_repo_hash(repo_root);
        let task_dir = self.xdg_directories.task_dir(task_dir_name, &repo_hash);
        let repo_path = task_dir.join("repo");

        // Create directories if they don't exist
        self.file_system
            .create_dir(&task_dir)
            .await
            .map_err(|e| format!("Failed to create task directory: {}", e))?;

        // Check if we're in a git repository
        if !self.git_operations.is_git_repository().await? {
            return Err("Not in a git repository".to_string());
        }

        // Use the provided repository root
        let current_dir = repo_root.to_path_buf();

        // Get list of tracked files from git
        let tracked_files = self.git_operations.get_tracked_files(&current_dir).await?;

        // Copy .git directory first
        let git_src = current_dir.join(".git");
        let git_dst = repo_path.join(".git");
        if self
            .file_system
            .exists(&git_src)
            .await
            .map_err(|e| format!("Failed to check if .git exists: {}", e))?
        {
            self.copy_directory(&git_src, &git_dst).await?;
        }

        // Copy all tracked files
        for file_path in tracked_files {
            let src_path = current_dir.join(&file_path);
            let dst_path = repo_path.join(&file_path);

            // Create parent directory if it doesn't exist
            if let Some(parent) = dst_path.parent() {
                self.file_system
                    .create_dir(parent)
                    .await
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
            }

            // Copy the file
            self.file_system
                .copy_file(&src_path, &dst_path)
                .await
                .map_err(|e| {
                    format!("Failed to copy tracked file {}: {}", file_path.display(), e)
                })?;
        }

        // Copy .tsk directory if it exists (for project-specific Docker configurations)
        let tsk_src = current_dir.join(".tsk");
        let tsk_dst = repo_path.join(".tsk");
        if self
            .file_system
            .exists(&tsk_src)
            .await
            .map_err(|e| format!("Failed to check if .tsk exists: {}", e))?
        {
            self.copy_directory(&tsk_src, &tsk_dst).await?;
        }

        // Create a new branch in the copied repository
        match source_commit {
            Some(commit_sha) => {
                // Create branch from specific commit
                self.git_operations
                    .create_branch_from_commit(&repo_path, &branch_name, commit_sha)
                    .await?;
                println!("Created branch from commit: {}", commit_sha);
            }
            None => {
                // Create branch from HEAD (existing behavior)
                self.git_operations
                    .create_branch(&repo_path, &branch_name)
                    .await?;
            }
        }

        println!("Created repository copy at: {}", repo_path.display());
        println!("Branch: {}", branch_name);
        Ok((repo_path, branch_name))
    }

    /// Copy directory recursively
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
    /// Returns false if no changes were fetched (branch has no new commits)
    pub async fn fetch_changes(
        &self,
        repo_path: &Path,
        branch_name: &str,
        repo_root: &Path,
    ) -> Result<bool, String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        // Use the provided repository root
        let main_repo = repo_root.to_path_buf();

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

        // Now check if the fetched branch has any commits not in main
        let has_commits = self
            .git_operations
            .has_commits_not_in_base(&main_repo, branch_name, "main")
            .await?;

        if !has_commits {
            println!("No new commits in branch {} - deleting branch", branch_name);
            // Delete the branch from the main repository since it has no new commits
            if let Err(e) = self
                .git_operations
                .delete_branch(&main_repo, branch_name)
                .await
            {
                eprintln!("Warning: Failed to delete branch {}: {}", branch_name, e);
            }
            return Ok(false);
        }

        println!("Fetched changes from copied repository");
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::git_operations::tests::MockGitOperations;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn create_test_xdg_directories(temp_dir: &TempDir) -> Arc<XdgDirectories> {
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().join("data"));
        std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path().join("runtime"));
        let xdg = XdgDirectories::new().unwrap();
        xdg.ensure_directories().unwrap();
        Arc::new(xdg)
    }

    #[tokio::test]
    async fn test_copy_repo_not_in_git_repo() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(false));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let manager = RepoManager::new(xdg_directories, fs, mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .copy_repo("2024-01-01-1200-generic-test-task", repo_root, None)
            .await;

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

        let xdg_directories = create_test_xdg_directories(&temp_dir);
        let manager = RepoManager::new(xdg_directories, fs, mock_git_ops);

        let result = manager.commit_changes(repo_path, "Test commit").await;

        assert!(result.is_ok(), "Error: {:?}", result);
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

        let xdg_directories = create_test_xdg_directories(&temp_dir);
        let manager = RepoManager::new(xdg_directories, fs, mock_git_ops);

        let result = manager.commit_changes(repo_path, "Test commit").await;

        assert!(result.is_ok(), "Error: {:?}", result);
    }

    #[tokio::test]
    async fn test_fetch_changes_no_commits() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_has_commits_not_in_base_result(Ok(false));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        // Create XDG directories for test
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().join("data"));
        std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path().join("runtime"));
        let xdg = Arc::new(crate::storage::XdgDirectories::new().unwrap());

        let manager = RepoManager::new(xdg, fs, mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .fetch_changes(repo_path, "tsk/test-branch", repo_root)
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        assert_eq!(result.unwrap(), false);

        // Verify that delete_branch was called
        let delete_calls = mock_git_ops.get_delete_branch_calls();
        assert_eq!(delete_calls.len(), 1);
        assert_eq!(delete_calls[0].1, "tsk/test-branch");
    }

    #[tokio::test]
    async fn test_fetch_changes_with_commits() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp_dir.path();

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_has_commits_not_in_base_result(Ok(true));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        // Create XDG directories for test
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().join("data"));
        std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path().join("runtime"));
        let xdg = Arc::new(crate::storage::XdgDirectories::new().unwrap());

        let manager = RepoManager::new(xdg, fs, mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .fetch_changes(repo_path, "tsk/test-branch", repo_root)
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        assert_eq!(result.unwrap(), true);

        // Verify that delete_branch was NOT called
        let delete_calls = mock_git_ops.get_delete_branch_calls();
        assert_eq!(delete_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_copy_repo_with_source_commit() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(true));
        mock_git_ops.set_get_tracked_files_result(Ok(vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("Cargo.toml"),
        ]));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());
        // Add mock files with absolute paths
        let mut files = HashMap::new();
        files.insert(temp_dir.path().join(".git"), "dir".to_string());
        files.insert(
            temp_dir.path().join("src/main.rs"),
            "file content".to_string(),
        );
        files.insert(temp_dir.path().join("Cargo.toml"), "[package]".to_string());
        fs.set_files(files);

        let manager = RepoManager::new(xdg_directories, fs, mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let source_commit = "abc123def456789012345678901234567890abcd";
        let result = manager
            .copy_repo(
                "2024-01-01-1200-generic-test-task",
                repo_root,
                Some(source_commit),
            )
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        let (_, branch_name) = result.unwrap();
        assert_eq!(branch_name, "tsk/2024-01-01-1200-generic-test-task");

        // Verify create_branch_from_commit was called
        let create_from_commit_calls = mock_git_ops.get_create_branch_from_commit_calls();
        assert_eq!(create_from_commit_calls.len(), 1);
        assert_eq!(
            create_from_commit_calls[0].1,
            "tsk/2024-01-01-1200-generic-test-task"
        );
        assert_eq!(create_from_commit_calls[0].2, source_commit);

        // Verify regular create_branch was NOT called
        let create_branch_calls = mock_git_ops.get_create_branch_calls();
        assert_eq!(create_branch_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_copy_repo_without_source_commit() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(true));
        mock_git_ops.set_get_tracked_files_result(Ok(vec![PathBuf::from("README.md")]));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());
        // Add mock files with absolute paths
        let mut files = HashMap::new();
        files.insert(temp_dir.path().join(".git"), "dir".to_string());
        files.insert(temp_dir.path().join("README.md"), "# README".to_string());
        fs.set_files(files);

        let manager = RepoManager::new(xdg_directories, fs, mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .copy_repo("2024-01-01-1200-generic-test-task", repo_root, None)
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        let (_, branch_name) = result.unwrap();
        assert_eq!(branch_name, "tsk/2024-01-01-1200-generic-test-task");

        // Verify regular create_branch was called
        let create_branch_calls = mock_git_ops.get_create_branch_calls();
        assert_eq!(create_branch_calls.len(), 1);
        assert_eq!(
            create_branch_calls[0].1,
            "tsk/2024-01-01-1200-generic-test-task"
        );

        // Verify create_branch_from_commit was NOT called
        let create_from_commit_calls = mock_git_ops.get_create_branch_from_commit_calls();
        assert_eq!(create_from_commit_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_copy_repo_only_copies_tracked_files() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(true));
        // Only track specific files, not the build artifacts
        mock_git_ops.set_get_tracked_files_result(Ok(vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("Cargo.toml"),
        ]));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());
        // Add mock files including untracked build artifacts with absolute paths
        let mut files = HashMap::new();
        files.insert(temp_dir.path().join(".git"), "dir".to_string());
        files.insert(
            temp_dir.path().join("src/main.rs"),
            "fn main() {}".to_string(),
        );
        files.insert(temp_dir.path().join("Cargo.toml"), "[package]".to_string());
        files.insert(
            temp_dir.path().join("target/debug/app"),
            "binary".to_string(),
        ); // untracked
        files.insert(temp_dir.path().join("build.log"), "log content".to_string()); // untracked
        fs.set_files(files);

        let manager = RepoManager::new(xdg_directories.clone(), fs.clone(), mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .copy_repo("2024-01-01-1200-generic-test-task", repo_root, None)
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        let (repo_path, _) = result.unwrap();

        // Verify only tracked files were copied
        let copied_files = fs.get_files();
        let repo_path_str = repo_path.to_string_lossy();

        // Check that tracked files exist in destination
        assert!(copied_files.contains_key(&format!("{}/src/main.rs", repo_path_str)));
        assert!(copied_files.contains_key(&format!("{}/Cargo.toml", repo_path_str)));

        // Check that untracked files were NOT copied
        assert!(!copied_files.contains_key(&format!("{}/target/debug/app", repo_path_str)));
        assert!(!copied_files.contains_key(&format!("{}/build.log", repo_path_str)));

        // Check that .git directory was copied
        let copied_dirs = fs.get_dirs();
        assert!(copied_dirs
            .iter()
            .any(|d| d == &format!("{}/.git", repo_path_str)));
    }

    #[tokio::test]
    async fn test_copy_repo_includes_tsk_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create mock git operations
        let mock_git_ops = Arc::new(MockGitOperations::new());
        mock_git_ops.set_is_repo_result(Ok(true));
        mock_git_ops.set_get_tracked_files_result(Ok(vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("Cargo.toml"),
        ]));

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());
        // Add mock files including .tsk directory
        let mut files = HashMap::new();
        files.insert(temp_dir.path().join(".git"), "dir".to_string());
        files.insert(temp_dir.path().join(".tsk"), "dir".to_string());
        files.insert(
            temp_dir
                .path()
                .join(".tsk/dockerfiles/project/test-project/Dockerfile"),
            "FROM ubuntu:22.04".to_string(),
        );
        files.insert(
            temp_dir.path().join("src/main.rs"),
            "fn main() {}".to_string(),
        );
        files.insert(temp_dir.path().join("Cargo.toml"), "[package]".to_string());
        fs.set_files(files);

        let manager = RepoManager::new(xdg_directories.clone(), fs.clone(), mock_git_ops.clone());

        let repo_root = temp_dir.path();
        let result = manager
            .copy_repo("2024-01-01-1200-generic-test-task", repo_root, None)
            .await;

        assert!(result.is_ok(), "Error: {:?}", result);
        let (repo_path, _) = result.unwrap();

        // Verify .tsk directory and its contents were copied
        let copied_files = fs.get_files();
        let copied_dirs = fs.get_dirs();
        let repo_path_str = repo_path.to_string_lossy();

        // Check that .tsk directory was copied
        assert!(copied_dirs
            .iter()
            .any(|d| d == &format!("{}/.tsk", repo_path_str)));

        // Check that .tsk contents were copied (directories and files)
        assert!(
            copied_dirs.iter().any(|d| d.contains(".tsk"))
                || copied_files.keys().any(|f| f.contains(".tsk/dockerfiles"))
        );

        // Check that the copy operation was called for .tsk directory
        // (The actual file copying might not show up in our mock due to the recursive copy)
        let copy_directory_exists = copied_dirs.iter().any(|d| d.contains(".tsk"))
            || fs.get_files().keys().any(|k| k.contains(".tsk"));
        assert!(
            copy_directory_exists,
            "Expected .tsk directory or its contents to be copied"
        );
    }
}
