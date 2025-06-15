use super::*;
use std::path::Path;
use tempfile::TempDir;

#[tokio::test]
async fn test_default_git_operations_is_git_repository() {
    let git_ops = DefaultGitOperations;

    // This will return false in a fresh temp directory
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let result = git_ops.is_git_repository().await;
    assert!(result.is_ok());
    assert!(!result.unwrap());

    // Restore original directory
    std::env::set_current_dir(original_dir).unwrap();
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
async fn test_mock_git_operations() {
    use super::tests::MockGitOperations;
    let mock = MockGitOperations::new();

    // Test is_git_repository
    mock.set_is_repo_result(Ok(true));
    assert_eq!(mock.is_git_repository().await.unwrap(), true);

    mock.set_is_repo_result(Ok(false));
    assert_eq!(mock.is_git_repository().await.unwrap(), false);

    // Test get_status
    mock.set_get_status_result(Ok("M file.txt\n".to_string()));
    let status = mock.get_status(Path::new("/test")).await.unwrap();
    assert_eq!(status, "M file.txt\n");

    // Test create_branch
    let result = mock
        .create_branch(Path::new("/test"), "feature-branch")
        .await;
    assert!(result.is_ok());

    // Verify the call was recorded
    let calls = mock.get_create_branch_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ("/test".to_string(), "feature-branch".to_string())
    );
}

#[tokio::test]
async fn test_mock_git_operations_commit_flow() {
    use super::tests::MockGitOperations;
    let mock = MockGitOperations::new();

    let repo_path = Path::new("/test/repo");

    // Simulate a commit flow
    mock.set_get_status_result(Ok("M file.txt\n".to_string()));
    let status = mock.get_status(repo_path).await.unwrap();
    assert!(!status.is_empty());

    // Add all changes
    let result = mock.add_all(repo_path).await;
    assert!(result.is_ok());

    // Commit
    let result = mock.commit(repo_path, "Test commit message").await;
    assert!(result.is_ok());

    // Verify calls were recorded
    assert_eq!(mock.get_get_status_calls().len(), 1);
    assert_eq!(mock.get_add_all_calls().len(), 1);
    assert_eq!(mock.get_commit_calls().len(), 1);

    let commit_calls = mock.get_commit_calls();
    assert_eq!(commit_calls[0].1, "Test commit message");
}

#[tokio::test]
async fn test_mock_git_operations_remote_operations() {
    use super::tests::MockGitOperations;
    let mock = MockGitOperations::new();

    let repo_path = Path::new("/test/repo");
    let remote_name = "origin";
    let remote_url = "https://github.com/test/repo.git";
    let branch_name = "main";

    // Add remote
    let result = mock.add_remote(repo_path, remote_name, remote_url).await;
    assert!(result.is_ok());

    // Fetch branch
    let result = mock.fetch_branch(repo_path, remote_name, branch_name).await;
    assert!(result.is_ok());

    // Remove remote
    let result = mock.remove_remote(repo_path, remote_name).await;
    assert!(result.is_ok());

    // Verify calls
    let add_remote_calls = mock.get_add_remote_calls();
    assert_eq!(add_remote_calls.len(), 1);
    assert_eq!(add_remote_calls[0].1, remote_name);
    assert_eq!(add_remote_calls[0].2, remote_url);

    let fetch_calls = mock.get_fetch_branch_calls();
    assert_eq!(fetch_calls.len(), 1);
    assert_eq!(fetch_calls[0].1, remote_name);
    assert_eq!(fetch_calls[0].2, branch_name);

    let remove_calls = mock.get_remove_remote_calls();
    assert_eq!(remove_calls.len(), 1);
    assert_eq!(remove_calls[0].1, remote_name);
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
async fn test_mock_get_current_commit() {
    use super::tests::MockGitOperations;
    let mock = MockGitOperations::new();

    // Test get_current_commit
    let commit_sha = mock.get_current_commit(Path::new("/test")).await.unwrap();
    assert_eq!(commit_sha, "abc123def456789012345678901234567890abcd");

    // Verify the call was recorded
    let calls = mock.get_get_current_commit_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "/test");

    // Test with error
    mock.set_get_current_commit_result(Err("Failed to get commit".to_string()));
    let result = mock.get_current_commit(Path::new("/test2")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_mock_create_branch_from_commit() {
    use super::tests::MockGitOperations;
    let mock = MockGitOperations::new();

    // Test create_branch_from_commit
    let result = mock
        .create_branch_from_commit(
            Path::new("/test"),
            "feature-branch",
            "abc123def456789012345678901234567890abcd",
        )
        .await;
    assert!(result.is_ok());

    // Verify the call was recorded
    let calls = mock.get_create_branch_from_commit_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "/test");
    assert_eq!(calls[0].1, "feature-branch");
    assert_eq!(calls[0].2, "abc123def456789012345678901234567890abcd");
}
