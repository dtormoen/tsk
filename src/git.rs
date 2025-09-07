use crate::context::AppContext;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};

/// Manages repository operations including copying, committing, and fetching changes.
///
/// This struct provides high-level repository management functionality, coordinating
/// between file system operations, git operations, and synchronization management.
pub struct RepoManager {
    ctx: AppContext,
}

impl RepoManager {
    /// Creates a new RepoManager from the application context.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context providing all required dependencies
    pub fn new(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    /// Copy repository for a task using the task ID and repository root
    ///
    /// Copies all non-ignored files from the working directory including:
    /// - Tracked files with their current working directory content (including unstaged changes)
    /// - Staged files (newly added files in the index)
    /// - Untracked files (not ignored)
    /// - The .git directory for full repository state
    /// - The .tsk directory for project-specific configurations
    ///
    /// This captures the complete state of the repository as shown by `git status`
    ///
    /// Returns the path to the copied repository and the branch name
    pub async fn copy_repo(
        &self,
        task_id: &str,
        repo_root: &Path,
        source_commit: Option<&str>,
        branch_name: &str,
    ) -> Result<(PathBuf, String), String> {
        // Use the task ID directly for the directory name
        let task_dir_name = task_id;
        let branch_name = branch_name.to_string();

        // Create the task directory structure in centralized location
        let repo_hash = crate::storage::get_repo_hash(repo_root);
        let task_dir = self.ctx.tsk_config().task_dir(task_dir_name, &repo_hash);
        let repo_path = task_dir.join("repo");

        // Create directories if they don't exist
        self.ctx
            .file_system()
            .create_dir(&task_dir)
            .await
            .map_err(|e| format!("Failed to create task directory: {e}"))?;

        // Check if the provided path is in a git repository
        if !self
            .ctx
            .git_operations()
            .is_git_repository(repo_root)
            .await?
        {
            return Err("Not in a git repository".to_string());
        }

        // Use the provided repository root
        let current_dir = repo_root.to_path_buf();

        // Get list of all files that should be copied:
        // 1. All tracked files (from working directory, including unstaged changes)
        // 2. All staged files (including newly added files in the index)
        // 3. All untracked files (not ignored)
        let all_files_to_copy = self
            .ctx
            .git_operations()
            .get_all_non_ignored_files(&current_dir)
            .await?;

        // Copy .git directory first
        let git_src = current_dir.join(".git");
        let git_dst = repo_path.join(".git");
        if self
            .ctx
            .file_system()
            .exists(&git_src)
            .await
            .map_err(|e| format!("Failed to check if .git exists: {e}"))?
        {
            self.ctx
                .file_system()
                .copy_dir(&git_src, &git_dst)
                .await
                .map_err(|e| format!("Failed to copy .git directory: {e}"))?;
        }

        // Create a new branch in the copied repository BEFORE copying files
        // This ensures that when source_commit is provided, the checkout doesn't
        // overwrite the working directory files we're about to copy
        match source_commit {
            Some(commit_sha) => {
                // Create branch from specific commit
                self.ctx
                    .git_operations()
                    .create_branch_from_commit(&repo_path, &branch_name, commit_sha)
                    .await?;
                println!("Created branch from commit: {commit_sha}");
            }
            None => {
                // Create branch from HEAD (existing behavior)
                self.ctx
                    .git_operations()
                    .create_branch(&repo_path, &branch_name)
                    .await?;
            }
        }

        // Copy all non-ignored files from the working directory
        // This happens AFTER branch creation to preserve unstaged changes
        for file_path in all_files_to_copy {
            // Remove trailing slash if present (git adds it for directories)
            let file_path_str = file_path.to_string_lossy();
            let file_path_clean = if let Some(stripped) = file_path_str.strip_suffix('/') {
                PathBuf::from(stripped)
            } else {
                file_path.clone()
            };

            let src_path = current_dir.join(&file_path_clean);
            let dst_path = repo_path.join(&file_path_clean);

            // Get metadata to check the actual entry type (not following symlinks)
            match tokio::fs::symlink_metadata(&src_path).await {
                Ok(metadata) => {
                    if metadata.is_dir() {
                        // It's an actual directory, copy it recursively
                        self.ctx
                            .file_system()
                            .copy_dir(&src_path, &dst_path)
                            .await
                            .map_err(|e| {
                                format!(
                                    "Failed to copy untracked directory {}: {e}",
                                    src_path.display()
                                )
                            })?;
                    } else if metadata.is_symlink() {
                        // It's a symlink - need special handling
                        // Create parent directory if it doesn't exist
                        if let Some(parent) = dst_path.parent() {
                            self.ctx
                                .file_system()
                                .create_dir(parent)
                                .await
                                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
                        }

                        // Read the symlink target and recreate it
                        let target = tokio::fs::read_link(&src_path).await.map_err(|e| {
                            format!("Failed to read symlink {}: {}", src_path.display(), e)
                        })?;

                        #[cfg(unix)]
                        tokio::fs::symlink(&target, &dst_path).await.map_err(|e| {
                            format!("Failed to create symlink {}: {}", dst_path.display(), e)
                        })?;

                        #[cfg(windows)]
                        {
                            // On Windows, determine if it's a file or directory symlink
                            if let Ok(target_meta) = tokio::fs::metadata(&src_path).await {
                                if target_meta.is_dir() {
                                    tokio::fs::symlink_dir(&target, &dst_path).await.map_err(
                                        |e| {
                                            format!(
                                                "Failed to create directory symlink {}: {}",
                                                dst_path.display(),
                                                e
                                            )
                                        },
                                    )?;
                                } else {
                                    tokio::fs::symlink_file(&target, &dst_path).await.map_err(
                                        |e| {
                                            format!(
                                                "Failed to create file symlink {}: {}",
                                                dst_path.display(),
                                                e
                                            )
                                        },
                                    )?;
                                }
                            } else {
                                // Default to file symlink if we can't determine
                                tokio::fs::symlink_file(&target, &dst_path)
                                    .await
                                    .map_err(|e| {
                                        format!(
                                            "Failed to create symlink {}: {}",
                                            dst_path.display(),
                                            e
                                        )
                                    })?;
                            }
                        }
                    } else {
                        // It's a regular file
                        // Create parent directory if it doesn't exist
                        if let Some(parent) = dst_path.parent() {
                            self.ctx
                                .file_system()
                                .create_dir(parent)
                                .await
                                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
                        }

                        // Copy the file
                        self.ctx
                            .file_system()
                            .copy_file(&src_path, &dst_path)
                            .await
                            .map_err(|e| {
                                format!("Failed to copy untracked file {}: {e}", src_path.display())
                            })?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // File doesn't exist - might be because git reported a directory with trailing slash
                    // that we already processed. Skip it.
                    continue;
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to get metadata for {}: {}",
                        src_path.display(),
                        e
                    ));
                }
            }
        }

        // Copy .tsk directory if it exists (for project-specific Docker configurations)
        let tsk_src = current_dir.join(".tsk");
        let tsk_dst = repo_path.join(".tsk");
        if self
            .ctx
            .file_system()
            .exists(&tsk_src)
            .await
            .map_err(|e| format!("Failed to check if .tsk exists: {e}"))?
        {
            self.ctx
                .file_system()
                .copy_dir(&tsk_src, &tsk_dst)
                .await
                .map_err(|e| format!("Failed to copy .tsk directory: {e}"))?;
        }

        println!("Created repository copy at: {}", repo_path.display());
        println!("Branch: {branch_name}");
        Ok((repo_path, branch_name))
    }

    /// Commit any uncommitted changes in the repository
    pub async fn commit_changes(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        // Check if there are any changes to commit
        let status_output = self.ctx.git_operations().get_status(repo_path).await?;

        if status_output.trim().is_empty() {
            println!("No changes to commit");
            return Ok(());
        }

        // Add all changes
        self.ctx.git_operations().add_all(repo_path).await?;

        // Commit changes
        self.ctx.git_operations().commit(repo_path, message).await?;

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

        // Synchronize git operations on the main repository
        self.ctx
            .git_sync_manager()
            .with_repo_lock(&main_repo, || async {
                self.ctx
                    .git_operations()
                    .add_remote(&main_repo, &remote_name, repo_path_str)
                    .await?;

                // Fetch the specific branch from the remote
                match self
                    .ctx
                    .git_operations()
                    .fetch_branch(&main_repo, &remote_name, branch_name)
                    .await
                {
                    Ok(_) => {
                        // Remove the temporary remote
                        self.ctx
                            .git_operations()
                            .remove_remote(&main_repo, &remote_name)
                            .await?;
                    }
                    Err(e) => {
                        // Remove the temporary remote before returning error
                        let _ = self
                            .ctx
                            .git_operations()
                            .remove_remote(&main_repo, &remote_name)
                            .await;
                        return Err(e);
                    }
                }
                Ok::<(), String>(())
            })
            .await?;

        // Now check if the fetched branch has any commits not in main
        let has_commits = self
            .ctx
            .git_operations()
            .has_commits_not_in_base(&main_repo, branch_name, "main")
            .await?;

        if !has_commits {
            println!("No new commits in branch {branch_name} - deleting branch");
            // Delete the branch from the main repository since it has no new commits
            if let Err(e) = self
                .ctx
                .git_operations()
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
    use crate::context::AppContext;
    use crate::test_utils::TestGitRepository;

    #[tokio::test]
    async fn test_copy_repo_not_in_git_repo() {
        let ctx = AppContext::builder().build();

        // Create a directory that is not a git repo
        let non_git_repo = TestGitRepository::new().unwrap();
        non_git_repo.setup_non_git_directory().unwrap();

        let manager = RepoManager::new(&ctx);

        let result = manager
            .copy_repo(
                "abcd1234",
                non_git_repo.path(),
                None,
                "tsk/test/test-task/abcd1234",
            )
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in a git repository");
    }

    #[tokio::test]
    async fn test_commit_changes_no_changes() {
        // Create a git repository with an initial commit
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let ctx = AppContext::builder().build();
        let manager = RepoManager::new(&ctx);

        // Test committing when there are no changes
        let result = manager
            .commit_changes(test_repo.path(), "Test commit")
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
    }

    #[tokio::test]
    async fn test_commit_changes_with_changes() {
        // Create a git repository with uncommitted changes
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Modify the existing file to create changes
        test_repo
            .create_file("README.md", "# Test Repository\n\nModified content\n")
            .unwrap();

        let ctx = AppContext::builder().build();
        let manager = RepoManager::new(&ctx);

        let result = manager
            .commit_changes(test_repo.path(), "Test commit")
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
    }

    #[tokio::test]
    async fn test_fetch_changes_no_commits() {
        let ctx = AppContext::builder().build();

        // Create main repository
        let main_repo = TestGitRepository::new().unwrap();
        main_repo.init().unwrap();
        // Configure git to use main as default branch
        main_repo
            .run_git_command(&["config", "init.defaultBranch", "main"])
            .unwrap();
        main_repo
            .run_git_command(&["checkout", "-b", "main"])
            .unwrap();
        // Create initial commit
        main_repo
            .create_file("README.md", "# Test Repository\n")
            .unwrap();
        main_repo.stage_all().unwrap();
        main_repo.commit("Initial commit").unwrap();

        // Create task repository (simulating a copied repository)
        let task_repo = TestGitRepository::new().unwrap();
        task_repo.init().unwrap();

        // Set up task repo to have main repo as origin
        task_repo
            .run_git_command(&[
                "remote",
                "add",
                "origin",
                main_repo.path().to_str().unwrap(),
            ])
            .unwrap();

        // Fetch main branch to task repo
        task_repo.run_git_command(&["fetch", "origin"]).unwrap();
        let main_branch = main_repo.current_branch().unwrap();
        task_repo
            .run_git_command(&[
                "checkout",
                "-b",
                &main_branch,
                &format!("origin/{main_branch}"),
            ])
            .unwrap();

        // Create a branch in task repo with no new commits
        let branch_name = "tsk/test/test-task/abcd1234";
        task_repo.checkout_new_branch(branch_name).unwrap();

        // Don't add any new commits - just the branch

        let manager = RepoManager::new(&ctx);

        // Fetch changes from task repo to main repo (should return false as there are no new commits)
        let result = manager
            .fetch_changes(task_repo.path(), branch_name, main_repo.path())
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
        assert!(!result.unwrap(), "Should return false when no new commits");

        // Verify the branch was cleaned up in main repo
        let main_branches = main_repo.branches().unwrap();
        assert!(
            !main_branches.contains(&branch_name.to_string()),
            "Branch should be cleaned up when no commits"
        );
    }

    #[tokio::test]
    async fn test_fetch_changes_with_commits() {
        let ctx = AppContext::builder().build();

        // Create main repository
        let main_repo = TestGitRepository::new().unwrap();
        main_repo.init().unwrap();
        // Configure git to use main as default branch
        main_repo
            .run_git_command(&["config", "init.defaultBranch", "main"])
            .unwrap();
        main_repo
            .run_git_command(&["checkout", "-b", "main"])
            .unwrap();
        // Create initial commit
        main_repo
            .create_file("README.md", "# Test Repository\n")
            .unwrap();
        main_repo.stage_all().unwrap();
        main_repo.commit("Initial commit").unwrap();

        // Create task repository (simulating a copied repository)
        let task_repo = TestGitRepository::new().unwrap();
        task_repo.init().unwrap();

        // Set up task repo to have main repo as origin
        task_repo
            .run_git_command(&[
                "remote",
                "add",
                "origin",
                main_repo.path().to_str().unwrap(),
            ])
            .unwrap();

        // Fetch main branch to task repo
        task_repo.run_git_command(&["fetch", "origin"]).unwrap();
        let main_branch = main_repo.current_branch().unwrap();
        task_repo
            .run_git_command(&[
                "checkout",
                "-b",
                &main_branch,
                &format!("origin/{main_branch}"),
            ])
            .unwrap();

        // Create a branch in task repo with new commits
        let branch_name = "tsk/test/test-task/efgh5678";
        task_repo.checkout_new_branch(branch_name).unwrap();

        // Add a new commit
        task_repo
            .create_file("new_feature.rs", "fn new_feature() {}")
            .unwrap();
        task_repo.stage_all().unwrap();
        task_repo.commit("Add new feature").unwrap();

        let manager = RepoManager::new(&ctx);

        // Fetch changes from task repo to main repo (should return true as there are new commits)
        let result = manager
            .fetch_changes(task_repo.path(), branch_name, main_repo.path())
            .await;

        assert!(result.is_ok(), "Error: {result:?}");
        assert!(result.unwrap(), "Should return true when new commits exist");

        // Verify the branch exists in main repo
        let main_branches = main_repo.branches().unwrap();
        assert!(
            main_branches.contains(&branch_name.to_string()),
            "Branch should exist after fetch"
        );
    }

    #[tokio::test]
    async fn test_copy_repo_with_source_commit() {
        let ctx = AppContext::builder().build();

        // Create a repository with multiple commits
        let test_repo = TestGitRepository::new().unwrap();
        let first_commit = test_repo.init_with_commit().unwrap();

        // Add more commits
        test_repo
            .create_file("feature1.rs", "fn feature1() {}")
            .unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add feature1").unwrap();

        test_repo
            .create_file("feature2.rs", "fn feature2() {}")
            .unwrap();
        test_repo.stage_all().unwrap();
        let _latest_commit = test_repo.commit("Add feature2").unwrap();

        let manager = RepoManager::new(&ctx);

        // Copy repo from the first commit
        let task_id = "efgh5678";
        let branch_name = "tsk/test/copy-repo-test/efgh5678";
        let result = manager
            .copy_repo(task_id, test_repo.path(), Some(&first_commit), branch_name)
            .await;

        assert!(result.is_ok());
        let (copied_path, returned_branch_name) = result.unwrap();

        assert_eq!(returned_branch_name, branch_name);
        assert!(copied_path.exists());

        // Verify the working directory contains all current files (preserving working directory state)
        // even though the branch was created from the first commit
        assert!(
            copied_path.join("feature1.rs").exists(),
            "Working directory files should be preserved"
        );
        assert!(
            copied_path.join("feature2.rs").exists(),
            "Working directory files should be preserved"
        );
        assert!(
            copied_path.join("README.md").exists(),
            "Original files should be preserved"
        );

        // Verify the branch was created from the first commit by checking git history
        let copied_repo = TestGitRepository::new().unwrap();
        let _ = std::fs::remove_dir_all(copied_repo.path());
        std::fs::rename(&copied_path, copied_repo.path()).unwrap();

        // The HEAD commit should be the first_commit (since we created branch from it)
        let head_commit = copied_repo.get_head_commit().unwrap();
        assert_eq!(
            head_commit, first_commit,
            "Branch should be created from the specified commit"
        );
    }

    #[tokio::test]
    async fn test_copy_repo_without_source_commit() {
        let ctx = AppContext::builder().build();

        // Create a repository with commits
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Add more files
        test_repo
            .create_file("feature.rs", "fn feature() {}")
            .unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add feature").unwrap();

        let manager = RepoManager::new(&ctx);

        // Copy repo without specifying source commit (should use HEAD)
        let task_id = "ijkl9012";
        let branch_name = "tsk/test/copy-repo-head/ijkl9012";
        let result = manager
            .copy_repo(task_id, test_repo.path(), None, branch_name)
            .await;

        assert!(result.is_ok());
        let (copied_path, returned_branch_name) = result.unwrap();

        assert_eq!(returned_branch_name, branch_name);
        assert!(copied_path.exists());

        // Verify the copied repo has all files from HEAD
        assert!(copied_path.join("README.md").exists());
        assert!(copied_path.join("feature.rs").exists());
    }

    #[tokio::test]
    async fn test_copy_repo_separates_tracked_and_untracked_files() {
        use crate::test_utils::create_files_with_gitignore;

        let ctx = AppContext::builder().build();

        // Create a repository with mixed file types
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init().unwrap();
        create_files_with_gitignore(&test_repo).unwrap();

        let manager = RepoManager::new(&ctx);

        // Copy the repository
        let task_id = "mnop3456";
        let branch_name = "tsk/test/tracked-untracked/mnop3456";
        let result = manager
            .copy_repo(task_id, test_repo.path(), None, branch_name)
            .await;

        assert!(result.is_ok());
        let (copied_path, _) = result.unwrap();

        // Verify tracked files are copied
        assert!(
            copied_path.join("src/main.rs").exists(),
            "Tracked files should be copied"
        );
        assert!(
            copied_path.join("Cargo.toml").exists(),
            "Tracked files should be copied"
        );
        assert!(
            copied_path.join(".gitignore").exists(),
            "Gitignore should be copied"
        );

        // Verify untracked files are copied
        assert!(
            copied_path.join("src/lib.rs").exists(),
            "Untracked files should be copied"
        );
        assert!(
            copied_path.join("README.md").exists(),
            "Untracked files should be copied"
        );

        // Verify ignored files are NOT copied
        assert!(
            !copied_path.join("debug.log").exists(),
            "Ignored files should not be copied"
        );
        assert!(
            !copied_path.join(".DS_Store").exists(),
            "Ignored files should not be copied"
        );
        assert!(
            !copied_path.join("target").exists(),
            "Ignored directories should not be copied"
        );
        assert!(
            !copied_path.join("tmp").exists(),
            "Ignored directories should not be copied"
        );

        // Verify .tsk directory IS copied even if it would normally be ignored
        assert!(
            copied_path.join(".tsk/config.json").exists(),
            ".tsk directory should always be copied"
        );
        assert!(
            copied_path
                .join(".tsk/dockerfiles/project/test/Dockerfile")
                .exists(),
            ".tsk directory should always be copied"
        );
    }

    #[tokio::test]
    async fn test_copy_repo_with_symlinks() {
        let ctx = AppContext::builder().build();

        // Create a repository with symlinks
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init().unwrap();

        // Create some regular files and directories
        test_repo
            .create_file("README.md", "# Test Repository\n")
            .unwrap();
        test_repo
            .create_file("src/main.rs", "fn main() {}")
            .unwrap();
        std::fs::create_dir_all(test_repo.path().join("docs")).unwrap();
        test_repo
            .create_file("docs/guide.md", "# User Guide\n")
            .unwrap();

        // Create symlinks
        #[cfg(unix)]
        {
            use std::os::unix::fs as unix_fs;

            // Create a symlink to a file
            unix_fs::symlink("README.md", test_repo.path().join("README_LINK.md")).unwrap();

            // Create a symlink to a directory
            unix_fs::symlink("docs", test_repo.path().join("documentation")).unwrap();

            // Create an absolute symlink to a file within the repo
            let readme_abs = test_repo.path().join("README.md");
            unix_fs::symlink(&readme_abs, test_repo.path().join("README_ABS.md")).unwrap();

            // Create a nested symlink (symlink inside a directory)
            unix_fs::symlink("../README.md", test_repo.path().join("docs/README_LINK.md")).unwrap();
        }

        #[cfg(windows)]
        {
            // On Windows, create file and directory symlinks
            std::fs::symlink_file("README.md", test_repo.path().join("README_LINK.md")).unwrap();

            std::fs::symlink_dir("docs", test_repo.path().join("documentation")).unwrap();
        }

        // Stage and commit all files including symlinks
        test_repo.stage_all().unwrap();
        test_repo.commit("Initial commit with symlinks").unwrap();

        // Also add an untracked symlink
        #[cfg(unix)]
        {
            use std::os::unix::fs as unix_fs;
            unix_fs::symlink("src/main.rs", test_repo.path().join("main_link.rs")).unwrap();
        }

        #[cfg(windows)]
        {
            std::fs::symlink_file("src/main.rs", test_repo.path().join("main_link.rs")).unwrap();
        }

        let manager = RepoManager::new(&ctx);

        // Copy the repository
        let task_id = "symlink123";
        let branch_name = "tsk/test/symlinks/symlink123";
        let result = manager
            .copy_repo(task_id, test_repo.path(), None, branch_name)
            .await;

        assert!(
            result.is_ok(),
            "Failed to copy repo with symlinks: {:?}",
            result
        );
        let (copied_path, _) = result.unwrap();

        // Verify regular files were copied
        assert!(copied_path.join("README.md").exists());
        assert!(copied_path.join("src/main.rs").exists());
        assert!(copied_path.join("docs/guide.md").exists());

        // Verify symlinks were preserved as symlinks
        #[cfg(unix)]
        {
            use std::fs;

            // Check tracked symlinks
            let readme_link_meta =
                fs::symlink_metadata(copied_path.join("README_LINK.md")).unwrap();
            assert!(
                readme_link_meta.is_symlink(),
                "README_LINK.md should be a symlink"
            );

            let docs_link_meta = fs::symlink_metadata(copied_path.join("documentation")).unwrap();
            assert!(
                docs_link_meta.is_symlink(),
                "documentation should be a symlink"
            );

            let readme_abs_meta = fs::symlink_metadata(copied_path.join("README_ABS.md")).unwrap();
            assert!(
                readme_abs_meta.is_symlink(),
                "README_ABS.md should be a symlink"
            );

            let nested_link_meta =
                fs::symlink_metadata(copied_path.join("docs/README_LINK.md")).unwrap();
            assert!(
                nested_link_meta.is_symlink(),
                "docs/README_LINK.md should be a symlink"
            );

            // Check untracked symlink
            let untracked_link_meta =
                fs::symlink_metadata(copied_path.join("main_link.rs")).unwrap();
            assert!(
                untracked_link_meta.is_symlink(),
                "main_link.rs should be a symlink"
            );

            // Verify symlink targets are correct
            let readme_target = fs::read_link(copied_path.join("README_LINK.md")).unwrap();
            assert_eq!(readme_target.to_string_lossy(), "README.md");

            let docs_target = fs::read_link(copied_path.join("documentation")).unwrap();
            assert_eq!(docs_target.to_string_lossy(), "docs");

            let nested_target = fs::read_link(copied_path.join("docs/README_LINK.md")).unwrap();
            assert_eq!(nested_target.to_string_lossy(), "../README.md");
        }

        #[cfg(windows)]
        {
            use std::fs;

            // Check symlinks on Windows
            let readme_link_meta =
                fs::symlink_metadata(copied_path.join("README_LINK.md")).unwrap();
            assert!(
                readme_link_meta.is_symlink(),
                "README_LINK.md should be a symlink"
            );

            let docs_link_meta = fs::symlink_metadata(copied_path.join("documentation")).unwrap();
            assert!(
                docs_link_meta.is_symlink(),
                "documentation should be a symlink"
            );

            let untracked_link_meta =
                fs::symlink_metadata(copied_path.join("main_link.rs")).unwrap();
            assert!(
                untracked_link_meta.is_symlink(),
                "main_link.rs should be a symlink"
            );
        }
    }

    #[tokio::test]
    async fn test_copy_repo_includes_tsk_directory() {
        let ctx = AppContext::builder().build();

        // Create a repository with .tsk directory
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create .tsk directory with various files
        test_repo
            .create_file(".tsk/config.json", r#"{"agent": "claude"}"#)
            .unwrap();
        test_repo
            .create_file(
                ".tsk/templates/feat.md",
                "# Feature Template\n{{DESCRIPTION}}",
            )
            .unwrap();
        test_repo
            .create_file(
                ".tsk/dockerfiles/project/myproject/Dockerfile",
                "FROM ubuntu:24.04\nRUN echo test",
            )
            .unwrap();

        // Add .tsk to .gitignore to test it's still copied
        test_repo.create_file(".gitignore", ".tsk/\n").unwrap();
        test_repo.stage_all().unwrap();
        test_repo
            .commit("Add .tsk directory and gitignore")
            .unwrap();

        let manager = RepoManager::new(&ctx);

        // Copy the repository
        let task_id = "qrst7890";
        let branch_name = "tsk/test/tsk-directory/qrst7890";
        let result = manager
            .copy_repo(task_id, test_repo.path(), None, branch_name)
            .await;

        assert!(result.is_ok());
        let (copied_path, _) = result.unwrap();

        // Verify .tsk directory and all its contents are copied
        assert!(
            copied_path.join(".tsk").exists(),
            ".tsk directory should be copied"
        );
        assert!(
            copied_path.join(".tsk/config.json").exists(),
            ".tsk/config.json should be copied"
        );
        assert!(
            copied_path.join(".tsk/templates/feat.md").exists(),
            ".tsk/templates should be copied"
        );
        assert!(
            copied_path
                .join(".tsk/dockerfiles/project/myproject/Dockerfile")
                .exists(),
            ".tsk/dockerfiles should be copied"
        );
    }

    #[tokio::test]
    async fn test_copy_repo_includes_unstaged_changes() {
        let ctx = AppContext::builder().build();

        // Create a repository with committed files
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create and commit a file with initial content
        test_repo
            .create_file("tracked.txt", "initial content")
            .unwrap();
        test_repo.stage_all().unwrap();
        test_repo.commit("Add tracked file").unwrap();

        // Modify the tracked file (unstaged change)
        test_repo
            .create_file("tracked.txt", "modified content - unstaged")
            .unwrap();

        // Create another file and stage it (staged change)
        test_repo
            .create_file("staged.txt", "staged content")
            .unwrap();
        test_repo.run_git_command(&["add", "staged.txt"]).unwrap();

        // Create an untracked file
        test_repo
            .create_file("untracked.txt", "untracked content")
            .unwrap();

        let manager = RepoManager::new(&ctx);

        // Copy the repository
        let task_id = "unstaged123";
        let branch_name = "tsk/test/unstaged-changes/unstaged123";
        let result = manager
            .copy_repo(task_id, test_repo.path(), None, branch_name)
            .await;

        assert!(result.is_ok());
        let (copied_path, _) = result.unwrap();

        // Verify the unstaged changes are included (working directory version)
        let tracked_content = std::fs::read_to_string(copied_path.join("tracked.txt")).unwrap();
        assert_eq!(
            tracked_content, "modified content - unstaged",
            "Unstaged changes should be copied (working directory version)"
        );

        // Verify staged file is copied
        let staged_content = std::fs::read_to_string(copied_path.join("staged.txt")).unwrap();
        assert_eq!(
            staged_content, "staged content",
            "Staged files should be copied"
        );

        // Verify untracked file is copied
        let untracked_content = std::fs::read_to_string(copied_path.join("untracked.txt")).unwrap();
        assert_eq!(
            untracked_content, "untracked content",
            "Untracked files should be copied"
        );
    }
}
