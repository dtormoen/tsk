//! Docker build lock management
//!
//! This module provides locking mechanisms to prevent concurrent Docker image builds
//! for the same image tag, ensuring builds are serialized while allowing different
//! images to build in parallel.

use crate::tui::events::{ServerEvent, ServerEventSender, emit_or_print};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// State of a Docker build lock
#[derive(Debug, Clone)]
pub enum BuildLockState {
    /// No build is currently in progress
    Idle,
    /// A build is currently in progress
    Building,
    /// Build completed (kept for a short time for caching)
    Completed,
}

/// Lock information for a specific image
#[derive(Debug)]
struct ImageLock {
    /// Current state of the build
    state: BuildLockState,
    /// Semaphore to control access (1 permit = exclusive build access)
    semaphore: Arc<Semaphore>,
    /// Number of tasks waiting for this lock
    waiting_count: usize,
}

impl ImageLock {
    fn new() -> Self {
        Self {
            state: BuildLockState::Idle,
            semaphore: Arc::new(Semaphore::new(1)),
            waiting_count: 0,
        }
    }
}

/// Guard that releases the build lock when dropped
pub struct BuildLockGuard {
    image_tag: String,
    manager: Arc<DockerBuildLockManager>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for BuildLockGuard {
    fn drop(&mut self) {
        // Mark the build as completed
        let mut locks = self.manager.locks.lock().unwrap();
        if let Some(lock) = locks.get_mut(&self.image_tag) {
            lock.state = BuildLockState::Completed;
        }
    }
}

/// Manages Docker build locks to prevent concurrent builds of the same image
pub struct DockerBuildLockManager {
    /// Map of image tags to their lock state
    locks: Arc<Mutex<HashMap<String, ImageLock>>>,
}

impl DockerBuildLockManager {
    /// Creates a new DockerBuildLockManager
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Acquires a build lock for the specified image tag
    ///
    /// This method will:
    /// 1. Return immediately if no build is in progress
    /// 2. Wait if another build is in progress for the same image
    /// 3. Return a guard that automatically releases the lock when dropped
    ///
    /// # Arguments
    /// * `image_tag` - The Docker image tag to lock
    ///
    /// # Returns
    /// A `BuildLockGuard` that holds the lock until dropped
    pub async fn acquire_build_lock(
        self: &Arc<Self>,
        image_tag: &str,
        event_sender: &Option<ServerEventSender>,
    ) -> BuildLockGuard {
        // Get or create the lock for this image
        let semaphore = {
            let mut locks = self.locks.lock().unwrap();
            let lock = locks
                .entry(image_tag.to_string())
                .or_insert_with(ImageLock::new);

            // Increment waiting count if build is in progress
            if matches!(lock.state, BuildLockState::Building) {
                lock.waiting_count += 1;

                // Log that we're waiting
                let waiting_position = lock.waiting_count;
                emit_or_print(
                    event_sender,
                    ServerEvent::StatusMessage(format!(
                        "Waiting for Docker build lock for image '{}' (position {} in queue)",
                        image_tag, waiting_position
                    )),
                );
            }

            Arc::clone(&lock.semaphore)
        };

        // Acquire the semaphore (will wait if another build is in progress)
        let permit = semaphore.acquire_owned().await.unwrap();

        // Update state to Building and decrement waiting count
        {
            let mut locks = self.locks.lock().unwrap();
            if let Some(lock) = locks.get_mut(image_tag) {
                if lock.waiting_count > 0 {
                    lock.waiting_count -= 1;
                }
                lock.state = BuildLockState::Building;
            }
        }

        BuildLockGuard {
            image_tag: image_tag.to_string(),
            manager: Arc::clone(self),
            _permit: permit,
        }
    }
}

impl Default for DockerBuildLockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_acquire_build_lock_exclusive() {
        let manager = Arc::new(DockerBuildLockManager::new());

        // Acquire first lock
        let _guard1 = manager.acquire_build_lock("test-image", &None).await;

        // Try to acquire second lock for same image (should wait)
        let manager_clone = Arc::clone(&manager);
        let acquire_task =
            tokio::spawn(
                async move { manager_clone.acquire_build_lock("test-image", &None).await },
            );

        // Give the task a moment to start waiting
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Drop first guard to release lock
        drop(_guard1);

        // Second acquisition should now succeed
        let result = timeout(Duration::from_secs(1), acquire_task).await;
        assert!(result.is_ok(), "Second lock acquisition should succeed");
    }

    #[tokio::test]
    async fn test_parallel_builds_different_images() {
        let manager = Arc::new(DockerBuildLockManager::new());

        // Acquire locks for different images simultaneously
        let manager1 = Arc::clone(&manager);
        let manager2 = Arc::clone(&manager);

        let (guard1, guard2) = tokio::join!(
            manager1.acquire_build_lock("image1", &None),
            manager2.acquire_build_lock("image2", &None)
        );

        // Both should succeed without waiting
        drop(guard1);
        drop(guard2);
    }

    #[tokio::test]
    async fn test_build_lock_state_transitions() {
        let manager = Arc::new(DockerBuildLockManager::new());

        // Acquire lock - should transition to Building
        let guard = manager.acquire_build_lock("test-image", &None).await;

        // Check that state is Building
        {
            let locks = manager.locks.lock().unwrap();
            if let Some(lock) = locks.get("test-image") {
                assert!(matches!(lock.state, BuildLockState::Building));
            }
        }

        // Release lock - should transition to Completed
        drop(guard);

        // Check that state is Completed
        {
            let locks = manager.locks.lock().unwrap();
            if let Some(lock) = locks.get("test-image") {
                assert!(matches!(lock.state, BuildLockState::Completed));
            }
        }
    }
}
