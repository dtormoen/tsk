use async_trait::async_trait;
use git2::{Repository, RepositoryOpenFlags};
use std::path::{Path, PathBuf};

#[async_trait]
pub trait GitOperations: Send + Sync {
    /// Check if the given path is within a git repository
    async fn is_git_repository(&self, repo_path: &Path) -> Result<bool, String>;

    /// Create a branch from HEAD
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The repository cannot be opened
    /// - HEAD cannot be resolved (e.g., empty repository with no commits)
    /// - The branch already exists
    /// - The working directory cannot be updated
    async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String>;

    async fn get_status(&self, repo_path: &Path) -> Result<String, String>;

    async fn add_all(&self, repo_path: &Path) -> Result<(), String>;

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String>;

    async fn add_remote(
        &self,
        repo_path: &Path,
        remote_name: &str,
        url: &str,
    ) -> Result<(), String>;

    async fn fetch_branch(
        &self,
        repo_path: &Path,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), String>;

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String>;

    /// Check if a branch has commits that are not in the base branch
    async fn has_commits_not_in_base(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<bool, String>;

    /// Delete a branch
    async fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String>;

    /// Get the current commit SHA
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The repository cannot be opened
    /// - HEAD cannot be resolved (e.g., empty repository with no commits)
    /// - The HEAD reference does not point to a valid commit
    async fn get_current_commit(&self, repo_path: &Path) -> Result<String, String>;

    /// Create a branch from a specific commit
    async fn create_branch_from_commit(
        &self,
        repo_path: &Path,
        branch_name: &str,
        commit_sha: &str,
    ) -> Result<(), String>;

    /// Get list of all non-ignored files in the working directory
    /// This includes tracked files (with or without modifications), staged files, and untracked files
    async fn get_all_non_ignored_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String>;

    /// Validate that a branch exists and points to an accessible commit
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The repository cannot be opened
    /// - The branch reference does not exist
    /// - The branch has no target commit
    /// - The target commit object is not accessible in the object database
    async fn validate_branch_accessible(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<(), String>;

    /// Clone a local repository without hardlinks
    ///
    /// This creates an optimized copy of the source repository at the destination path.
    /// The clone operation repacks objects efficiently, typically resulting in 1-2 pack files
    /// instead of preserving fragmented pack structure.
    ///
    /// # Arguments
    ///
    /// * `source_repo_path` - Path to the source repository to clone
    /// * `destination_path` - Path where the cloned repository will be created
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source path is invalid or not a git repository
    /// - The destination path cannot be created
    /// - The clone operation fails
    async fn clone_local(
        &self,
        source_repo_path: &Path,
        destination_path: &Path,
    ) -> Result<(), String>;
}

pub struct DefaultGitOperations;

impl DefaultGitOperations {}

#[async_trait]
impl GitOperations for DefaultGitOperations {
    async fn is_git_repository(&self, repo_path: &Path) -> Result<bool, String> {
        match Repository::open_ext(repo_path, RepositoryOpenFlags::empty(), &[] as &[&Path]) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {e}"))?;

                let commit = head
                    .peel_to_commit()
                    .map_err(|e| format!("Failed to get commit from HEAD: {e}"))?;

                repo.branch(&branch_name, &commit, false)
                    .map_err(|e| format!("Failed to create branch: {e}"))?;

                repo.set_head(&format!("refs/heads/{branch_name}"))
                    .map_err(|e| format!("Failed to checkout branch: {e}"))?;

                repo.checkout_head(None)
                    .map_err(|e| format!("Failed to update working directory: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_status(&self, repo_path: &Path) -> Result<String, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<String, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let statuses = repo
                    .statuses(None)
                    .map_err(|e| format!("Failed to get repository status: {e}"))?;

                let mut result = String::new();

                for entry in statuses.iter() {
                    let status = entry.status();
                    if let Some(path) = entry.path() {
                        let status_char = if status.is_wt_new() {
                            "??"
                        } else if status.contains(git2::Status::INDEX_NEW) {
                            "A"
                        } else if status.contains(git2::Status::INDEX_MODIFIED)
                            || status.contains(git2::Status::WT_MODIFIED)
                        {
                            "M"
                        } else if status.contains(git2::Status::INDEX_DELETED)
                            || status.contains(git2::Status::WT_DELETED)
                        {
                            "D"
                        } else if status.contains(git2::Status::INDEX_RENAMED)
                            || status.contains(git2::Status::WT_RENAMED)
                        {
                            "R"
                        } else if status.contains(git2::Status::CONFLICTED) {
                            "C"
                        } else {
                            continue;
                        };

                        result.push_str(&format!("{status_char} {path}\n"));
                    }
                }

                Ok(result)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn add_all(&self, repo_path: &Path) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {e}"))?;

                index
                    .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                    .map_err(|e| format!("Failed to add files to index: {e}"))?;

                index
                    .write()
                    .map_err(|e| format!("Failed to write index: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let message = message.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {e}"))?;

                let tree_id = index
                    .write_tree()
                    .map_err(|e| format!("Failed to write tree: {e}"))?;

                let tree = repo
                    .find_tree(tree_id)
                    .map_err(|e| format!("Failed to find tree: {e}"))?;

                let signature = repo
                    .signature()
                    .map_err(|e| format!("Failed to get signature: {e}"))?;

                let parent_commit = match repo.head() {
                    Ok(head) => Some(
                        head.peel_to_commit()
                            .map_err(|e| format!("Failed to get parent commit: {e}"))?,
                    ),
                    Err(_) => None,
                };

                let parents = if let Some(ref parent) = parent_commit {
                    vec![parent]
                } else {
                    vec![]
                };

                repo.commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    &parents,
                )
                .map_err(|e| format!("Failed to create commit: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn add_remote(
        &self,
        repo_path: &Path,
        remote_name: &str,
        url: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            let url = url.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let result = repo.remote(&remote_name, &url);
                match result {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        if e.code() == git2::ErrorCode::Exists {
                            Ok(())
                        } else {
                            Err(format!("Failed to add remote: {e}"))
                        }
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn fetch_branch(
        &self,
        repo_path: &Path,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let remote = repo
                    .find_remote(&remote_name)
                    .map_err(|e| format!("Failed to find remote: {e}"))?;

                let url = remote
                    .url()
                    .ok_or_else(|| "Remote has no URL".to_string())?;

                let output = std::process::Command::new("git")
                    .current_dir(&repo_path)
                    .arg("fetch")
                    .arg(url)
                    .arg(format!("refs/heads/{branch_name}:refs/heads/{branch_name}"))
                    .output()
                    .map_err(|e| format!("Failed to execute git fetch: {e}"))?;

                if !output.status.success() {
                    return Err(format!(
                        "Failed to fetch changes: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                repo.remote_delete(&remote_name)
                    .map_err(|e| format!("Failed to remove temporary remote: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn has_commits_not_in_base(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<bool, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            let base_branch = base_branch.to_owned();
            move || -> Result<bool, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                // Get the branch reference
                let branch_ref = format!("refs/heads/{branch_name}");
                let branch = repo
                    .find_reference(&branch_ref)
                    .map_err(|e| format!("Failed to find branch {branch_name}: {e}"))?;

                let branch_oid = branch
                    .target()
                    .ok_or_else(|| format!("Branch {branch_name} has no target"))?;

                // Get the base branch reference
                let base_ref = format!("refs/heads/{base_branch}");
                let base = repo
                    .find_reference(&base_ref)
                    .map_err(|e| format!("Failed to find base branch {base_branch}: {e}"))?;

                let base_oid = base
                    .target()
                    .ok_or_else(|| format!("Base branch {base_branch} has no target"))?;

                // If they point to the same commit, there are no unique commits
                if branch_oid == base_oid {
                    return Ok(false);
                }

                // Check if the branch commit is reachable from the base branch
                // If it is, then there are no unique commits in the branch
                match repo.graph_descendant_of(base_oid, branch_oid) {
                    Ok(true) => Ok(false), // branch is behind base, no unique commits
                    Ok(false) => Ok(true), // branch has commits not in base
                    Err(_) => {
                        // If we can't determine the relationship, assume there are commits
                        // This is safer than assuming there aren't
                        Ok(true)
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut branch = repo
                    .find_branch(&branch_name, git2::BranchType::Local)
                    .map_err(|e| format!("Failed to find branch {branch_name}: {e}"))?;

                branch
                    .delete()
                    .map_err(|e| format!("Failed to delete branch {branch_name}: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_current_commit(&self, repo_path: &Path) -> Result<String, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<String, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {e}"))?;

                let commit = head
                    .peel_to_commit()
                    .map_err(|e| format!("Failed to get commit from HEAD: {e}"))?;

                Ok(commit.id().to_string())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn create_branch_from_commit(
        &self,
        repo_path: &Path,
        branch_name: &str,
        commit_sha: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            let commit_sha = commit_sha.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let oid = git2::Oid::from_str(&commit_sha)
                    .map_err(|e| format!("Invalid commit SHA: {e}"))?;

                let commit = repo
                    .find_commit(oid)
                    .map_err(|e| format!("Failed to find commit {commit_sha}: {e}"))?;

                repo.branch(&branch_name, &commit, false)
                    .map_err(|e| format!("Failed to create branch: {e}"))?;

                repo.set_head(&format!("refs/heads/{branch_name}"))
                    .map_err(|e| format!("Failed to checkout branch: {e}"))?;

                // Force update the working directory to match the commit
                let mut checkout_opts = git2::build::CheckoutBuilder::new();
                checkout_opts.force();
                repo.checkout_head(Some(&mut checkout_opts))
                    .map_err(|e| format!("Failed to update working directory: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_all_non_ignored_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<Vec<PathBuf>, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut opts = git2::StatusOptions::new();
                opts.include_untracked(true)
                    .include_ignored(false)
                    .include_unmodified(true);

                let statuses = repo
                    .statuses(Some(&mut opts))
                    .map_err(|e| format!("Failed to get repository status: {e}"))?;

                let mut files = Vec::new();

                for entry in statuses.iter() {
                    let status = entry.status();
                    if let Some(path) = entry.path()
                        && !status.is_ignored()
                    {
                        files.push(PathBuf::from(path));
                    }
                }

                Ok(files)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn validate_branch_accessible(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                // Check branch exists
                let branch_ref = format!("refs/heads/{branch_name}");
                let reference = repo
                    .find_reference(&branch_ref)
                    .map_err(|e| format!("Branch '{}' not found: {}", branch_name, e))?;

                // Check branch points to valid commit
                let oid = reference
                    .target()
                    .ok_or_else(|| format!("Branch '{}' has no target", branch_name))?;

                // Try to find the commit - this validates object accessibility
                repo.find_commit(oid).map_err(|e| {
                    format!(
                        "Branch '{}' points to inaccessible commit {}: {}",
                        branch_name, oid, e
                    )
                })?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn clone_local(
        &self,
        source_repo_path: &Path,
        destination_path: &Path,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let source_repo_path = source_repo_path.to_owned();
            let destination_path = destination_path.to_owned();
            move || -> Result<(), String> {
                let mut builder = git2::build::RepoBuilder::new();
                builder.clone_local(git2::build::CloneLocal::NoLinks);

                builder
                    .clone(
                        source_repo_path.to_str().ok_or("Invalid source path")?,
                        &destination_path,
                    )
                    .map_err(|e| format!("Failed to clone repository: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_is_git_repository() {
        let git_ops = DefaultGitOperations;

        // Test with a directory that is not a git repository
        let non_git_dir = TempDir::new().unwrap();
        let is_repo = git_ops.is_git_repository(non_git_dir.path()).await.unwrap();
        assert!(!is_repo, "Non-git directory should return false");

        // Test with a valid git repository
        let git_dir = TempDir::new().unwrap();
        git2::Repository::init(git_dir.path()).unwrap();
        let is_repo = git_ops.is_git_repository(git_dir.path()).await.unwrap();
        assert!(is_repo, "Git repository should return true");

        // Test with a subdirectory inside a git repository
        let subdir = git_dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        let is_repo = git_ops.is_git_repository(&subdir).await.unwrap();
        assert!(
            is_repo,
            "Subdirectory inside git repository should return true"
        );
    }

    #[tokio::test]
    async fn test_default_git_operations_with_real_repo() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a real git repository
        let repo = git2::Repository::init(repo_path).unwrap();

        // Test get_status on empty repo
        let status = git_ops.get_status(repo_path).await.unwrap();
        assert_eq!(status, "");

        // Create a test file
        std::fs::write(repo_path.join("test.txt"), "Hello, world!").unwrap();

        // Test get_status with untracked file
        let status = git_ops.get_status(repo_path).await.unwrap();
        assert!(status.contains("?? test.txt"));

        // Test add_all
        git_ops.add_all(repo_path).await.unwrap();

        // Test get_status after add
        let status = git_ops.get_status(repo_path).await.unwrap();
        assert!(status.contains("A test.txt"));

        // Configure git user for commit
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Test commit
        git_ops.commit(repo_path, "Initial commit").await.unwrap();

        // Test get_status after commit
        let status = git_ops.get_status(repo_path).await.unwrap();
        assert_eq!(status, "");

        // Test create_branch
        git_ops
            .create_branch(repo_path, "test-branch")
            .await
            .unwrap();

        // Verify we're on the new branch
        let head = repo.head().unwrap();
        let branch_name = head.shorthand().unwrap();
        assert_eq!(branch_name, "test-branch");
    }

    #[tokio::test]
    async fn test_default_git_operations_remotes() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a git repository
        git2::Repository::init(repo_path).unwrap();

        // Test add_remote
        git_ops
            .add_remote(repo_path, "origin", "https://github.com/test/repo.git")
            .await
            .unwrap();

        // Test adding the same remote again (should not error)
        git_ops
            .add_remote(repo_path, "origin", "https://github.com/test/repo.git")
            .await
            .unwrap();

        // Test remove_remote
        git_ops.remove_remote(repo_path, "origin").await.unwrap();
    }

    #[tokio::test]
    async fn test_get_current_commit() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a real git repository
        let repo = git2::Repository::init(repo_path).unwrap();

        // Configure git user for commit
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create and commit a file
        std::fs::write(repo_path.join("test.txt"), "Hello, world!").unwrap();
        git_ops.add_all(repo_path).await.unwrap();
        git_ops.commit(repo_path, "Initial commit").await.unwrap();

        // Get the current commit
        let commit_sha = git_ops.get_current_commit(repo_path).await.unwrap();
        assert!(!commit_sha.is_empty());
        assert_eq!(commit_sha.len(), 40); // SHA should be 40 characters

        // Verify it's the same as what git2 reports
        let head = repo.head().unwrap();
        let head_commit = head.peel_to_commit().unwrap();
        assert_eq!(commit_sha, head_commit.id().to_string());
    }

    #[tokio::test]
    async fn test_create_branch_from_commit() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a real git repository
        let repo = git2::Repository::init(repo_path).unwrap();

        // Configure git user for commit
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create first commit
        std::fs::write(repo_path.join("file1.txt"), "First file").unwrap();
        git_ops.add_all(repo_path).await.unwrap();
        git_ops.commit(repo_path, "First commit").await.unwrap();

        // Get the first commit SHA
        let first_commit_sha = git_ops.get_current_commit(repo_path).await.unwrap();

        // Create second commit
        std::fs::write(repo_path.join("file2.txt"), "Second file").unwrap();
        git_ops.add_all(repo_path).await.unwrap();
        git_ops.commit(repo_path, "Second commit").await.unwrap();

        // Create a branch from the first commit
        git_ops
            .create_branch_from_commit(repo_path, "feature-from-first", &first_commit_sha)
            .await
            .unwrap();

        // Verify we're on the new branch
        let head = repo.head().unwrap();
        let branch_name = head.shorthand().unwrap();
        assert_eq!(branch_name, "feature-from-first");

        // Verify the branch is at the first commit
        let current_commit = head.peel_to_commit().unwrap();
        assert_eq!(current_commit.id().to_string(), first_commit_sha);

        // Verify the second file doesn't exist in the working directory
        assert!(!repo_path.join("file2.txt").exists());
        assert!(repo_path.join("file1.txt").exists());
    }

    #[tokio::test]
    async fn test_get_current_commit_empty_repository() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a git repository without any commits
        git2::Repository::init(repo_path).unwrap();

        // Attempt to get current commit from empty repository
        let result = git_ops.get_current_commit(repo_path).await;

        // Verify it returns an error (not a panic)
        assert!(
            result.is_err(),
            "get_current_commit should return error on empty repository"
        );

        // Verify the error message is descriptive
        let error_message = result.unwrap_err();
        assert!(
            error_message.contains("Failed to get HEAD")
                || error_message.contains("Failed to get commit from HEAD"),
            "Error should mention HEAD failure, got: {error_message}"
        );
    }

    #[tokio::test]
    async fn test_create_branch_empty_repository() {
        let git_ops = DefaultGitOperations;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a git repository without any commits
        git2::Repository::init(repo_path).unwrap();

        // Attempt to create a branch in empty repository
        let result = git_ops.create_branch(repo_path, "test-branch").await;

        // Verify it returns an error (not a panic)
        assert!(
            result.is_err(),
            "create_branch should return error on empty repository"
        );

        // Verify the error message is descriptive
        let error_message = result.unwrap_err();
        assert!(
            error_message.contains("Failed to get commit from HEAD")
                || error_message.contains("Failed to get HEAD")
                || error_message.contains("unborn"),
            "Error should mention HEAD or unborn branch, got: {error_message}"
        );
    }

    #[tokio::test]
    async fn test_clone_local() {
        let git_ops = DefaultGitOperations;

        // Create source repository with commits
        let source_dir = TempDir::new().unwrap();
        let source_path = source_dir.path();
        let source_repo = git2::Repository::init(source_path).unwrap();

        // Configure git user
        let mut config = source_repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create first commit
        std::fs::write(source_path.join("file1.txt"), "First file").unwrap();
        git_ops.add_all(source_path).await.unwrap();
        git_ops.commit(source_path, "First commit").await.unwrap();
        let first_commit_sha = git_ops.get_current_commit(source_path).await.unwrap();

        // Create second commit
        std::fs::write(source_path.join("file2.txt"), "Second file").unwrap();
        git_ops.add_all(source_path).await.unwrap();
        git_ops.commit(source_path, "Second commit").await.unwrap();
        let second_commit_sha = git_ops.get_current_commit(source_path).await.unwrap();

        // Clone the repository
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().join("cloned_repo");
        git_ops.clone_local(source_path, &dest_path).await.unwrap();

        // Verify cloned repository exists and is a valid git repository
        assert!(dest_path.exists(), "Cloned repository should exist");
        assert!(
            dest_path.join(".git").exists(),
            "Cloned repository should have .git directory"
        );

        let cloned_repo = git2::Repository::open(&dest_path).unwrap();

        // Verify the cloned repository has the same commits
        let cloned_head = cloned_repo.head().unwrap();
        let cloned_commit = cloned_head.peel_to_commit().unwrap();
        assert_eq!(
            cloned_commit.id().to_string(),
            second_commit_sha,
            "Cloned repository should have the same HEAD commit"
        );

        // Verify both files exist in the cloned repository
        assert!(
            dest_path.join("file1.txt").exists(),
            "First file should exist in cloned repo"
        );
        assert!(
            dest_path.join("file2.txt").exists(),
            "Second file should exist in cloned repo"
        );

        // Verify we can access the first commit in the cloned repository
        let first_commit_oid = git2::Oid::from_str(&first_commit_sha).unwrap();
        let first_commit = cloned_repo.find_commit(first_commit_oid).unwrap();
        assert_eq!(
            first_commit.id().to_string(),
            first_commit_sha,
            "Should be able to access first commit in cloned repo"
        );

        // Verify pack file optimization (should have 1-2 pack files, not 30+)
        let pack_dir = dest_path.join(".git/objects/pack");
        if pack_dir.exists() {
            let pack_files: Vec<_> = std::fs::read_dir(&pack_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s == "pack")
                        .unwrap_or(false)
                })
                .collect();

            assert!(
                pack_files.len() <= 2,
                "Cloned repository should have at most 2 pack files, found {}",
                pack_files.len()
            );
        }
    }
}
