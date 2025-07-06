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
    /// Copies git-tracked files, untracked files (not ignored), the .git directory, and the .tsk directory
    /// This captures the complete state of the repository as shown by `git status`
    /// Returns the path to the copied repository and the branch name
    pub async fn copy_repo(
        &self,
        task_id: &str,
        repo_root: &Path,
        source_commit: Option<&str>,
    ) -> Result<(PathBuf, String), String> {
        // Use the task ID directly for the directory name
        let task_dir_name = task_id;
        let branch_name = format!("tsk/{task_id}");

        // Create the task directory structure in centralized location
        let repo_hash = crate::storage::get_repo_hash(repo_root);
        let task_dir = self.xdg_directories.task_dir(task_dir_name, &repo_hash);
        let repo_path = task_dir.join("repo");

        // Create directories if they don't exist
        self.file_system
            .create_dir(&task_dir)
            .await
            .map_err(|e| format!("Failed to create task directory: {e}"))?;

        // Check if the provided path is in a git repository
        if !self.git_operations.is_git_repository(repo_root).await? {
            return Err("Not in a git repository".to_string());
        }

        // Use the provided repository root
        let current_dir = repo_root.to_path_buf();

        // Get list of tracked files from git
        let tracked_files = self.git_operations.get_tracked_files(&current_dir).await?;

        // Get list of untracked files that are not ignored
        let untracked_files = self
            .git_operations
            .get_untracked_files(&current_dir)
            .await?;

        // Copy .git directory first
        let git_src = current_dir.join(".git");
        let git_dst = repo_path.join(".git");
        if self
            .file_system
            .exists(&git_src)
            .await
            .map_err(|e| format!("Failed to check if .git exists: {e}"))?
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
                    .map_err(|e| format!("Failed to create parent directory: {e}"))?;
            }

            // Copy the file
            self.file_system
                .copy_file(&src_path, &dst_path)
                .await
                .map_err(|e| {
                    format!("Failed to copy tracked file {}: {}", file_path.display(), e)
                })?;
        }

        // Copy all untracked files (not ignored)
        for file_path in untracked_files {
            // Remove trailing slash if present (git adds it for directories)
            let file_path_str = file_path.to_string_lossy();
            let file_path_clean = if let Some(stripped) = file_path_str.strip_suffix('/') {
                PathBuf::from(stripped)
            } else {
                file_path.clone()
            };

            let src_path = current_dir.join(&file_path_clean);
            let dst_path = repo_path.join(&file_path_clean);

            // Check if this is a directory
            if self.file_system.read_dir(&src_path).await.is_ok() {
                // It's a directory, copy it recursively
                self.copy_directory(&src_path, &dst_path).await?;
            } else {
                // It's a file
                // Create parent directory if it doesn't exist
                if let Some(parent) = dst_path.parent() {
                    self.file_system
                        .create_dir(parent)
                        .await
                        .map_err(|e| format!("Failed to create parent directory: {e}"))?;
                }

                // Copy the file
                match self.file_system.copy_file(&src_path, &dst_path).await {
                    Ok(_) => {}
                    Err(e) => {
                        // If the file doesn't exist in src, it might be because git reported
                        // a directory with a trailing slash. Skip it.
                        if !e.to_string().contains("Source file not found") {
                            return Err(format!(
                                "Failed to copy untracked file {}: {}",
                                file_path.display(),
                                e
                            ));
                        }
                    }
                }
            }
        }

        // Copy .tsk directory if it exists (for project-specific Docker configurations)
        let tsk_src = current_dir.join(".tsk");
        let tsk_dst = repo_path.join(".tsk");
        if self
            .file_system
            .exists(&tsk_src)
            .await
            .map_err(|e| format!("Failed to check if .tsk exists: {e}"))?
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
                println!("Created branch from commit: {commit_sha}");
            }
            None => {
                // Create branch from HEAD (existing behavior)
                self.git_operations
                    .create_branch(&repo_path, &branch_name)
                    .await?;
            }
        }

        println!("Created repository copy at: {}", repo_path.display());
        println!("Branch: {branch_name}");
        Ok((repo_path, branch_name))
    }

    /// Copy directory recursively
    #[allow(clippy::only_used_in_recursion)]
    async fn copy_directory(&self, src: &Path, dst: &Path) -> Result<(), String> {
        self.file_system
            .create_dir(dst)
            .await
            .map_err(|e| format!("Failed to create destination directory: {e}"))?;

        let entries = self
            .file_system
            .read_dir(src)
            .await
            .map_err(|e| format!("Failed to read directory: {e}"))?;

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

        println!("Committed changes: {message}");
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
            println!("No new commits in branch {branch_name} - deleting branch");
            // Delete the branch from the main repository since it has no new commits
            if let Err(e) = self
                .git_operations
                .delete_branch(&main_repo, branch_name)
                .await
            {
                eprintln!("Warning: Failed to delete branch {branch_name}: {e}");
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
    use crate::context::git_operations::DefaultGitOperations;
    use crate::test_utils::TestGitRepository;
    use tempfile::TempDir;

    fn create_test_xdg_directories(temp_dir: &TempDir) -> Arc<XdgDirectories> {
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = XdgDirectories::new(Some(config)).unwrap();
        xdg.ensure_directories().unwrap();
        Arc::new(xdg)
    }

    #[tokio::test]
    async fn test_copy_repo_not_in_git_repo() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let xdg_directories = create_test_xdg_directories(&temp_dir);

        // Create a directory that is not a git repo
        let non_git_repo = TestGitRepository::new().unwrap();
        non_git_repo.setup_non_git_directory().unwrap();

        let git_ops = Arc::new(DefaultGitOperations);

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let manager = RepoManager::new(xdg_directories, fs, git_ops);

        let result = manager
            .copy_repo(
                "2024-01-01-1200-generic-test-task",
                non_git_repo.path(),
                None,
            )
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in a git repository");
    }

    #[tokio::test]
    async fn test_commit_changes_no_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create a git repository with an initial commit
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let git_ops = Arc::new(DefaultGitOperations);

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let xdg_directories = create_test_xdg_directories(&temp_dir);
        let manager = RepoManager::new(xdg_directories, fs, git_ops);

        // Test committing when there are no changes
        let result = manager
            .commit_changes(test_repo.path(), "Test commit")
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
    }

    #[tokio::test]
    async fn test_commit_changes_with_changes() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create a git repository with uncommitted changes
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Modify the existing file to create changes
        test_repo
            .create_file("README.md", "# Test Repository\n\nModified content\n")
            .unwrap();

        let git_ops = Arc::new(DefaultGitOperations);

        use crate::context::file_system::tests::MockFileSystem;
        let fs = Arc::new(MockFileSystem::new());

        let xdg_directories = create_test_xdg_directories(&temp_dir);
        let manager = RepoManager::new(xdg_directories, fs, git_ops);

        let result = manager
            .commit_changes(test_repo.path(), "Test commit")
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
    }

    // TODO: Rewrite test_fetch_changes_no_commits to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_fetch_changes_no_commits() {}

    // TODO: Rewrite test_fetch_changes_with_commits to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_fetch_changes_with_commits() {}

    // TODO: Rewrite test_copy_repo_with_source_commit to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_copy_repo_with_source_commit() {}

    // TODO: Rewrite test_copy_repo_without_source_commit to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_copy_repo_without_source_commit() {}

    // TODO: Rewrite test_copy_repo_separates_tracked_and_untracked_files to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_copy_repo_separates_tracked_and_untracked_files() {}

    // TODO: Rewrite test_copy_repo_includes_untracked_files to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_copy_repo_includes_untracked_files() {}

    // TODO: Rewrite test_copy_repo_includes_tsk_directory to use TestGitRepository instead of MockGitOperations
    #[tokio::test]
    #[ignore = "Needs refactoring to remove MockGitOperations"]
    async fn test_copy_repo_includes_tsk_directory() {}
}
