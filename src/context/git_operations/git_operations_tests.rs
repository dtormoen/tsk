use super::*;
use std::path::Path;
use tempfile::TempDir;

#[tokio::test]
async fn test_default_git_operations_is_git_repository() {
    let git_ops = DefaultGitOperations;

    // This will return false in a fresh temp directory
    let temp_dir = TempDir::new().unwrap();
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let result = git_ops.is_git_repository().await;
    assert!(result.is_ok());
    assert!(!result.unwrap());
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
