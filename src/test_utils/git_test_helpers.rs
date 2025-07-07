use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use crate::context::AppContext;
use crate::context::git_operations::DefaultGitOperations;
use crate::storage::XdgConfig;
use crate::test_utils::TestGitRepository;

/// Creates a test context with a real git repository for testing git operations.
/// Returns (TempDir, repo_path, TestGitRepository, AppContext)
pub fn create_test_context_with_git() -> Result<(TempDir, PathBuf, TestGitRepository, AppContext)> {
    let temp_dir = TempDir::new().context("Failed to create temp dir")?;

    // Create test git repository
    let test_repo = TestGitRepository::new()?;
    test_repo.init_with_commit()?;

    let repo_path = test_repo.path().to_path_buf();

    // Create XDG directories in temp dir
    let config = XdgConfig::with_paths(
        temp_dir.path().join("data"),
        temp_dir.path().join("runtime"),
        temp_dir.path().join("config"),
    );

    let xdg = crate::storage::XdgDirectories::new(Some(config))
        .context("Failed to create XDG directories")?;
    xdg.ensure_directories()
        .context("Failed to ensure XDG directories")?;

    // Build AppContext with real git operations
    let ctx = AppContext::builder()
        .with_xdg_directories(Arc::new(xdg))
        .with_git_operations(Arc::new(DefaultGitOperations))
        .build();

    Ok((temp_dir, repo_path, test_repo, ctx))
}

/// Sets up two git repositories with a remote relationship for testing fetch operations.
/// Returns (main_repo, task_repo)
pub fn setup_two_repos_with_remote() -> Result<(TestGitRepository, TestGitRepository)> {
    let main_repo = TestGitRepository::new()?;
    main_repo.init_with_commit()?;

    let task_repo = TestGitRepository::new()?;
    task_repo.init()?;

    // Add main repo as remote to task repo
    task_repo.run_git_command(&[
        "remote",
        "add",
        "origin",
        main_repo.path().to_str().unwrap(),
    ])?;

    // Fetch from main repo to set up tracking
    task_repo.run_git_command(&["fetch", "origin"])?;

    Ok((main_repo, task_repo))
}

/// Creates a repository with a mix of tracked, untracked, and ignored files for testing.
pub fn create_files_with_gitignore(repo: &TestGitRepository) -> Result<()> {
    // Create .gitignore
    repo.create_file(".gitignore", "target/\n*.log\n.DS_Store\ntmp/\n")?;

    // Create tracked files
    repo.create_file(
        "src/main.rs",
        "fn main() {\n    println!(\"Hello, world!\");\n}\n",
    )?;
    repo.create_file(
        "Cargo.toml",
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )?;

    // Stage and commit tracked files
    repo.stage_all()?;
    repo.commit("Add initial files")?;

    // Create untracked files that should be included
    repo.create_file("src/lib.rs", "pub fn lib_function() {}\n")?;
    repo.create_file("README.md", "# Test Project\n")?;

    // Create ignored files that should NOT be included
    repo.create_file("debug.log", "Debug log content\n")?;
    repo.create_file(".DS_Store", "Mac system file\n")?;
    repo.create_file("target/debug/binary", "Binary content\n")?;
    repo.create_file("tmp/temp.txt", "Temporary file\n")?;

    // Create .tsk directory that should always be included
    repo.create_file(".tsk/config.json", "{\"test\": true}\n")?;
    repo.create_file(
        ".tsk/dockerfiles/project/test/Dockerfile",
        "FROM ubuntu:22.04\n",
    )?;

    Ok(())
}

/// Creates a repository with multiple branches for testing branch operations.
pub fn setup_repo_with_branches(repo: &TestGitRepository, branches: Vec<&str>) -> Result<()> {
    // Initialize with first commit
    repo.init_with_commit()?;

    let main_branch = repo.current_branch()?;

    for branch_name in branches {
        // Create and checkout new branch
        repo.checkout_new_branch(branch_name)?;

        // Add unique content to branch
        repo.create_file(
            &format!("{}.txt", branch_name),
            &format!("Content for {} branch\n", branch_name),
        )?;
        repo.stage_all()?;
        repo.commit(&format!("Add content for {} branch", branch_name))?;

        // Go back to main branch
        repo.checkout_branch(&main_branch)?;
    }

    Ok(())
}

/// Extension trait to add git command helpers to TestGitRepository
pub trait TestGitRepositoryExt {
    /// Runs a git command and returns the output
    fn run_git_command(&self, args: &[&str]) -> Result<String>;

    /// Adds a file to .gitignore
    fn add_to_gitignore(&self, pattern: &str) -> Result<()>;

    /// Creates a tag at the current commit
    fn create_tag(&self, tag_name: &str) -> Result<()>;

    /// Gets the remote URL for a given remote name
    fn get_remote_url(&self, remote_name: &str) -> Result<String>;
}

impl TestGitRepositoryExt for TestGitRepository {
    fn run_git_command(&self, args: &[&str]) -> Result<String> {
        use std::process::Command;

        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git command failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn add_to_gitignore(&self, pattern: &str) -> Result<()> {
        let gitignore_path = self.path().join(".gitignore");
        let current_content = if gitignore_path.exists() {
            std::fs::read_to_string(&gitignore_path)?
        } else {
            String::new()
        };

        let new_content = if current_content.is_empty() {
            pattern.to_string()
        } else if current_content.ends_with('\n') {
            format!("{}{}", current_content, pattern)
        } else {
            format!("{}\n{}", current_content, pattern)
        };

        std::fs::write(&gitignore_path, new_content)?;
        Ok(())
    }

    fn create_tag(&self, tag_name: &str) -> Result<()> {
        self.run_git_command(&["tag", tag_name])?;
        Ok(())
    }

    fn get_remote_url(&self, remote_name: &str) -> Result<String> {
        let output = self.run_git_command(&["remote", "get-url", remote_name])?;
        Ok(output.trim().to_string())
    }
}
