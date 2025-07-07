use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A test utility that creates and manages a temporary git repository for testing.
///
/// This struct provides methods for common git operations needed in tests,
/// automatically cleans up on drop, and ensures tests have isolated git repositories.
pub struct TestGitRepository {
    #[allow(dead_code)] // This field keeps the TempDir alive for the lifetime of the repository
    temp_dir: TempDir,
    repo_path: PathBuf,
}

impl TestGitRepository {
    /// Creates a new temporary directory for testing.
    /// Does not initialize git repository - call `init()` or `init_with_commit()` for that.
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp dir")?;
        let repo_path = temp_dir.path().to_path_buf();

        Ok(Self {
            temp_dir,
            repo_path,
        })
    }

    /// Returns the path to the repository.
    pub fn path(&self) -> &Path {
        &self.repo_path
    }

    /// Initializes a new git repository without any commits.
    pub fn init(&self) -> Result<()> {
        self.run_git_command(&["init"])
            .context("Failed to initialize git repository")?;

        // Set local git config to avoid using global config
        self.run_git_command(&["config", "user.email", "test@example.com"])?;
        self.run_git_command(&["config", "user.name", "Test User"])?;

        Ok(())
    }

    /// Initializes a new git repository with an initial commit.
    /// Returns the commit SHA of the initial commit.
    pub fn init_with_commit(&self) -> Result<String> {
        self.init()?;

        // Create an initial file
        self.create_file("README.md", "# Test Repository\n")?;
        self.stage_all()?;
        self.commit("Initial commit")
    }

    /// Creates a file in the repository with the given content.
    pub fn create_file(&self, path: &str, content: &str) -> Result<()> {
        let file_path = self.repo_path.join(path);

        // Create parent directories if needed
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directories for {}", path))?;
        }

        fs::write(&file_path, content).with_context(|| format!("Failed to write file {}", path))?;

        Ok(())
    }

    /// Reads a file from the repository.
    pub fn read_file(&self, path: &str) -> Result<String> {
        let file_path = self.repo_path.join(path);
        fs::read_to_string(&file_path).with_context(|| format!("Failed to read file {}", path))
    }

    /// Stages all changes in the repository.
    pub fn stage_all(&self) -> Result<()> {
        self.run_git_command(&["add", "-A"])
            .context("Failed to stage files")?;
        Ok(())
    }

    /// Creates a commit with the given message.
    /// Returns the commit SHA.
    pub fn commit(&self, message: &str) -> Result<String> {
        self.run_git_command(&["commit", "-m", message])
            .context("Failed to create commit")?;

        self.get_current_commit()
    }

    /// Gets the SHA of the current commit.
    pub fn get_current_commit(&self) -> Result<String> {
        let output = self
            .run_git_command(&["rev-parse", "HEAD"])
            .context("Failed to get current commit")?;

        Ok(output.trim().to_string())
    }

    /// Gets the name of the current branch.
    pub fn current_branch(&self) -> Result<String> {
        let output = self
            .run_git_command(&["branch", "--show-current"])
            .context("Failed to get current branch")?;

        Ok(output.trim().to_string())
    }

    /// Creates and checks out a new branch.
    pub fn checkout_new_branch(&self, branch_name: &str) -> Result<()> {
        self.run_git_command(&["checkout", "-b", branch_name])
            .context("Failed to create and checkout new branch")?;
        Ok(())
    }

    /// Checks out an existing branch.
    pub fn checkout_branch(&self, branch_name: &str) -> Result<()> {
        self.run_git_command(&["checkout", branch_name])
            .context("Failed to checkout branch")?;
        Ok(())
    }

    /// Gets the git status output.
    pub fn status(&self) -> Result<String> {
        self.run_git_command(&["status", "--porcelain"])
            .context("Failed to get git status")
    }

    /// Gets list of all branches.
    pub fn branches(&self) -> Result<Vec<String>> {
        let output = self
            .run_git_command(&["branch", "--format=%(refname:short)"])
            .context("Failed to list branches")?;

        Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    /// Sets up a repository with uncommitted changes.
    /// Creates both staged and unstaged files.
    pub fn setup_with_uncommitted_changes(&self) -> Result<()> {
        self.init_with_commit()?;

        // Create an unstaged file
        self.create_file("unstaged.txt", "This file is not staged\n")?;

        // Create and stage a file
        self.create_file("staged.txt", "This file is staged\n")?;
        self.run_git_command(&["add", "staged.txt"])?;

        // Modify the README (which was committed earlier)
        self.create_file("README.md", "# Test Repository\n\nModified content\n")?;

        Ok(())
    }

    /// Sets up a repository with multiple branches.
    pub fn setup_with_branches(&self, branches: Vec<&str>) -> Result<()> {
        self.init_with_commit()?;

        let main_branch = self.current_branch()?;

        for branch in branches {
            self.checkout_new_branch(branch)?;
            self.create_file(
                &format!("{}.txt", branch),
                &format!("Content for {}\n", branch),
            )?;
            self.stage_all()?;
            self.commit(&format!("Add {}.txt", branch))?;
            self.checkout_branch(&main_branch)?;
        }

        Ok(())
    }

    /// Creates a regular directory without git initialization.
    pub fn setup_non_git_directory(&self) -> Result<()> {
        // Just create a file to ensure the directory exists
        self.create_file("file.txt", "Not a git repository\n")?;
        Ok(())
    }

    /// Runs a git command in the repository directory.
    pub fn run_git_command(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git command failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_repository() -> Result<()> {
        let repo = TestGitRepository::new()?;
        repo.init()?;

        assert!(repo.path().exists());
        assert!(repo.path().join(".git").exists());

        Ok(())
    }

    #[test]
    fn test_init_with_commit() -> Result<()> {
        let repo = TestGitRepository::new()?;
        let commit_sha = repo.init_with_commit()?;

        assert!(!commit_sha.is_empty());
        assert_eq!(commit_sha.len(), 40); // Git SHA-1 is 40 characters

        let readme_content = repo.read_file("README.md")?;
        assert_eq!(readme_content, "# Test Repository\n");

        Ok(())
    }

    #[test]
    fn test_create_and_commit_file() -> Result<()> {
        let repo = TestGitRepository::new()?;
        repo.init()?;

        repo.create_file("test.txt", "Hello, World!")?;
        repo.stage_all()?;
        let commit_sha = repo.commit("Add test file")?;

        assert!(!commit_sha.is_empty());

        let content = repo.read_file("test.txt")?;
        assert_eq!(content, "Hello, World!");

        Ok(())
    }

    #[test]
    fn test_branch_operations() -> Result<()> {
        let repo = TestGitRepository::new()?;
        repo.init_with_commit()?;

        let initial_branch = repo.current_branch()?;
        assert!(initial_branch == "main" || initial_branch == "master");

        repo.checkout_new_branch("feature")?;
        assert_eq!(repo.current_branch()?, "feature");

        repo.checkout_branch(&initial_branch)?;
        assert_eq!(repo.current_branch()?, initial_branch);

        Ok(())
    }

    #[test]
    fn test_setup_with_uncommitted_changes() -> Result<()> {
        let repo = TestGitRepository::new()?;
        repo.setup_with_uncommitted_changes()?;

        let status = repo.status()?;
        assert!(status.contains("unstaged.txt"));
        assert!(status.contains("staged.txt"));
        assert!(status.contains("README.md"));

        Ok(())
    }

    #[test]
    fn test_setup_with_branches() -> Result<()> {
        let repo = TestGitRepository::new()?;
        repo.setup_with_branches(vec!["feature", "bugfix", "develop"])?;

        let branches = repo.branches()?;
        assert!(branches.iter().any(|b| b == "feature"));
        assert!(branches.iter().any(|b| b == "bugfix"));
        assert!(branches.iter().any(|b| b == "develop"));

        Ok(())
    }
}
