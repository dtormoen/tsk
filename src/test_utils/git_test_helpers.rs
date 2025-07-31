//! Git test helper functions for setting up test repositories with specific configurations.

use anyhow::Result;

use crate::test_utils::TestGitRepository;

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
        "FROM ubuntu:24.04\n",
    )?;

    Ok(())
}
