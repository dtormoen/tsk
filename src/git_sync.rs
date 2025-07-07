use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Git operation synchronization to prevent concurrent operations on the same repository
pub struct GitSyncManager {
    /// Map of repository paths to their locks
    /// Uses RwLock for the map to allow concurrent reads
    repo_locks: Arc<RwLock<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

impl GitSyncManager {
    pub fn new() -> Self {
        Self {
            repo_locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a lock for a repository path
    pub async fn get_repo_lock(&self, repo_path: &Path) -> Arc<Mutex<()>> {
        // Normalize the path to ensure consistency
        let canonical_path = match repo_path.canonicalize() {
            Ok(path) => path,
            Err(_) => repo_path.to_path_buf(), // Fallback if canonicalize fails
        };

        // First try to get with read lock
        {
            let locks = self.repo_locks.read().await;
            if let Some(lock) = locks.get(&canonical_path) {
                return Arc::clone(lock);
            }
        }

        // Need to create new lock, acquire write lock
        let mut locks = self.repo_locks.write().await;

        // Double-check pattern - another thread might have created it
        if let Some(lock) = locks.get(&canonical_path) {
            return Arc::clone(lock);
        }

        // Create new lock
        let new_lock = Arc::new(Mutex::new(()));
        locks.insert(canonical_path.clone(), Arc::clone(&new_lock));
        new_lock
    }

    /// Perform a git operation with synchronization
    pub async fn with_repo_lock<F, Fut, T>(&self, repo_path: &Path, operation: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let lock = self.get_repo_lock(repo_path).await;
        let _guard = lock.lock().await;
        operation().await
    }
}

impl Default for GitSyncManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_git_sync_manager_prevents_concurrent_access() {
        let sync_manager = Arc::new(GitSyncManager::new());
        let test_path = PathBuf::from("/test/repo");
        let test_path_clone1 = test_path.clone();
        let test_path_clone2 = test_path.clone();

        let counter = Arc::new(Mutex::new(0));
        let counter_clone1 = Arc::clone(&counter);
        let counter_clone2 = Arc::clone(&counter);

        let sync_clone1 = Arc::clone(&sync_manager);
        let sync_clone2 = Arc::clone(&sync_manager);

        // Start two concurrent operations
        let handle1 = tokio::spawn(async move {
            sync_clone1
                .with_repo_lock(&test_path_clone1, || async {
                    let mut count = counter_clone1.lock().await;
                    *count += 1;
                    let current = *count;
                    drop(count);

                    // Simulate work
                    sleep(Duration::from_millis(50)).await;

                    // Check that count hasn't changed during our work
                    let count = counter_clone1.lock().await;
                    assert_eq!(*count, current, "Count changed during operation!");
                })
                .await;
        });

        let handle2 = tokio::spawn(async move {
            // Small delay to ensure first operation starts first
            sleep(Duration::from_millis(10)).await;

            sync_clone2
                .with_repo_lock(&test_path_clone2, || async {
                    let mut count = counter_clone2.lock().await;
                    *count += 1;
                    let current = *count;
                    drop(count);

                    // Simulate work
                    sleep(Duration::from_millis(50)).await;

                    // Check that count hasn't changed during our work
                    let count = counter_clone2.lock().await;
                    assert_eq!(*count, current, "Count changed during operation!");
                })
                .await;
        });

        // Wait for both to complete
        handle1.await.unwrap();
        handle2.await.unwrap();

        // Verify both operations completed
        let final_count = *counter.lock().await;
        assert_eq!(final_count, 2);
    }

    #[tokio::test]
    async fn test_different_repos_can_run_concurrently() {
        let sync_manager = Arc::new(GitSyncManager::new());
        let start = std::time::Instant::now();

        let sync_clone1 = Arc::clone(&sync_manager);
        let sync_clone2 = Arc::clone(&sync_manager);

        // Start operations on different repositories
        let handle1 = tokio::spawn(async move {
            sync_clone1
                .with_repo_lock(&PathBuf::from("/repo1"), || async {
                    sleep(Duration::from_millis(100)).await;
                })
                .await;
        });

        let handle2 = tokio::spawn(async move {
            sync_clone2
                .with_repo_lock(&PathBuf::from("/repo2"), || async {
                    sleep(Duration::from_millis(100)).await;
                })
                .await;
        });

        // Wait for both to complete
        handle1.await.unwrap();
        handle2.await.unwrap();

        let elapsed = start.elapsed();

        // If they ran concurrently, should take ~100ms, not 200ms
        assert!(
            elapsed.as_millis() < 150,
            "Operations on different repos should run concurrently"
        );
    }
}
