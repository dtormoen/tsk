use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Git operation synchronization to prevent concurrent operations on the same repository.
///
/// Uses a dual-lock architecture:
/// 1. In-process `tokio::Mutex` prevents thundering herd within one process
/// 2. Cross-process `flock(2)` protects against multiple TSK processes on the same repo
pub struct GitSyncManager {
    repo_locks: Arc<RwLock<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

impl GitSyncManager {
    pub fn new() -> Self {
        Self {
            repo_locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Perform a git operation with both in-process and cross-process synchronization.
    pub async fn with_repo_lock<F, Fut, T>(&self, repo_path: &Path, operation: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let canonical_path = canonicalize_path(repo_path);

        let lock = self.get_or_create_lock(&canonical_path).await;
        let _guard = lock.lock().await;

        let lock_path = canonical_path.join(".git").join("tsk.lock");
        let _flock_file = acquire_flock(lock_path.clone()).await.unwrap_or_else(|e| {
            panic!(
                "Failed to acquire repository file lock at {}: {}",
                lock_path.display(),
                e
            )
        });

        operation().await
    }

    /// Get or create an in-process lock for a canonicalized repository path.
    async fn get_or_create_lock(&self, canonical_path: &Path) -> Arc<Mutex<()>> {
        {
            let locks = self.repo_locks.read().await;
            if let Some(lock) = locks.get(canonical_path) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.repo_locks.write().await;

        // Double-check: another task may have inserted while we waited for the write lock
        if let Some(lock) = locks.get(canonical_path) {
            return Arc::clone(lock);
        }

        let new_lock = Arc::new(Mutex::new(()));
        locks.insert(canonical_path.to_path_buf(), Arc::clone(&new_lock));
        new_lock
    }
}

impl Default for GitSyncManager {
    fn default() -> Self {
        Self::new()
    }
}

fn canonicalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Acquires an exclusive `flock(2)` on the given lock file path.
///
/// Opens (or creates) the lock file and calls `flock(LOCK_EX)` in a blocking
/// context. Returns the open `File` whose lifetime controls the lock duration —
/// dropping the file closes the fd and releases the lock.
async fn acquire_flock(lock_path: PathBuf) -> std::io::Result<File> {
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        // SAFETY: `file.as_raw_fd()` returns a valid fd from the open File above.
        // `libc::flock` is a well-defined POSIX syscall that takes a valid fd.
        let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(file)
    })
    .await
    .expect("flock spawn_blocking task panicked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestGitRepository;
    use std::time::Duration;
    use tokio::time::sleep;

    fn create_temp_repo() -> TestGitRepository {
        let repo = TestGitRepository::new().unwrap();
        repo.init().unwrap();
        repo
    }

    #[tokio::test]
    async fn test_git_sync_manager_prevents_concurrent_access() {
        let sync_manager = Arc::new(GitSyncManager::new());
        let repo = create_temp_repo();
        let repo_path: PathBuf = repo.path().to_path_buf();
        let repo_path_clone1 = repo_path.clone();
        let repo_path_clone2 = repo_path.clone();

        let counter = Arc::new(Mutex::new(0));
        let counter_clone1 = Arc::clone(&counter);
        let counter_clone2 = Arc::clone(&counter);

        let sync_clone1 = Arc::clone(&sync_manager);
        let sync_clone2 = Arc::clone(&sync_manager);

        let handle1 = tokio::spawn(async move {
            sync_clone1
                .with_repo_lock(&repo_path_clone1, || async {
                    let mut count = counter_clone1.lock().await;
                    *count += 1;
                    let current = *count;
                    drop(count);

                    sleep(Duration::from_millis(50)).await;

                    let count = counter_clone1.lock().await;
                    assert_eq!(*count, current, "Count changed during operation!");
                })
                .await;
        });

        let handle2 = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;

            sync_clone2
                .with_repo_lock(&repo_path_clone2, || async {
                    let mut count = counter_clone2.lock().await;
                    *count += 1;
                    let current = *count;
                    drop(count);

                    sleep(Duration::from_millis(50)).await;

                    let count = counter_clone2.lock().await;
                    assert_eq!(*count, current, "Count changed during operation!");
                })
                .await;
        });

        handle1.await.unwrap();
        handle2.await.unwrap();

        let final_count = *counter.lock().await;
        assert_eq!(final_count, 2);
    }

    #[tokio::test]
    async fn test_different_repos_can_run_concurrently() {
        let sync_manager = Arc::new(GitSyncManager::new());
        let repo1 = create_temp_repo();
        let repo2 = create_temp_repo();
        let repo_path1 = repo1.path().to_path_buf();
        let repo_path2 = repo2.path().to_path_buf();
        let start = std::time::Instant::now();

        let sync_clone1 = Arc::clone(&sync_manager);
        let sync_clone2 = Arc::clone(&sync_manager);

        let handle1 = tokio::spawn(async move {
            sync_clone1
                .with_repo_lock(&repo_path1, || async {
                    sleep(Duration::from_millis(100)).await;
                })
                .await;
        });

        let handle2 = tokio::spawn(async move {
            sync_clone2
                .with_repo_lock(&repo_path2, || async {
                    sleep(Duration::from_millis(100)).await;
                })
                .await;
        });

        handle1.await.unwrap();
        handle2.await.unwrap();

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 150,
            "Operations on different repos should run concurrently"
        );
    }

    #[tokio::test]
    async fn test_lock_file_created_at_expected_location() {
        let sync_manager = GitSyncManager::new();
        let repo = create_temp_repo();
        let repo_path = repo.path().to_path_buf();
        let expected_lock = repo.path().join(".git").join("tsk.lock");

        assert!(!expected_lock.exists(), "Lock file should not exist yet");

        sync_manager.with_repo_lock(&repo_path, || async {}).await;

        assert!(
            expected_lock.exists(),
            "Lock file should exist at <repo>/.git/tsk.lock"
        );
    }
}
