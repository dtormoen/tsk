use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

/// Result of executing a job
#[derive(Debug, Clone)]
pub struct JobResult {
    /// Unique identifier for the job
    pub job_id: String,
    /// Whether the job completed successfully
    pub success: bool,
    /// Optional message about the job result
    pub message: Option<String>,
}

/// Error that can occur during job execution
#[derive(Debug)]
pub struct JobError {
    pub message: String,
}

impl From<String> for JobError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for JobError {}

/// Trait for jobs that can be executed by the worker pool
pub trait AsyncJob: Send + 'static {
    /// Execute the job asynchronously
    fn execute(self) -> impl std::future::Future<Output = Result<JobResult, JobError>> + Send;

    /// Get the unique identifier for this job
    fn job_id(&self) -> String;
}

/// Handle to a submitted job
pub struct JobHandle {
    /// Unique identifier for the job
    #[allow(dead_code)]
    pub job_id: String,
}

/// Generic worker pool for executing async jobs with concurrency control
pub struct WorkerPool<T: AsyncJob> {
    /// Maximum number of concurrent workers
    workers: usize,
    /// Semaphore to control concurrency
    semaphore: Arc<Semaphore>,
    /// Set of active job handles
    active_jobs: Arc<Mutex<JoinSet<Result<JobResult, JobError>>>>,
    /// Flag to indicate if the pool is shutting down
    shutting_down: Arc<Mutex<bool>>,
    /// Phantom data to hold the job type
    _phantom: std::marker::PhantomData<T>,
}

impl<T: AsyncJob> WorkerPool<T> {
    /// Create a new worker pool with the specified number of workers
    pub fn new(workers: usize) -> Self {
        Self {
            workers,
            semaphore: Arc::new(Semaphore::new(workers)),
            active_jobs: Arc::new(Mutex::new(JoinSet::new())),
            shutting_down: Arc::new(Mutex::new(false)),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get the total number of workers in the pool
    pub fn total_workers(&self) -> usize {
        self.workers
    }

    /// Get the number of currently active workers (executing jobs)
    pub fn active_workers(&self) -> usize {
        self.workers - self.semaphore.available_permits()
    }

    /// Get the number of available workers (not currently executing jobs)
    pub fn available_workers(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Try to submit a job without waiting for a worker to become available
    ///
    /// Returns None if no workers are available, otherwise returns a JobHandle.
    pub async fn try_submit(&self, job: T) -> Result<Option<JobHandle>, JobError> {
        // Check if we're shutting down
        if *self.shutting_down.lock().await {
            return Err(JobError::from("Worker pool is shutting down".to_string()));
        }

        // Try to acquire a permit without waiting
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Ok(None),
        };

        // Get the job ID before moving the job
        let job_id = job.job_id();

        // Add the job to the active jobs set for tracking
        let mut jobs = self.active_jobs.lock().await;
        jobs.spawn(async move {
            // Hold the permit for the duration of the job
            let _permit = permit;

            // Execute the job
            job.execute().await
        });
        drop(jobs);

        Ok(Some(JobHandle { job_id }))
    }

    /// Clean up completed jobs from the internal tracking
    ///
    /// Returns the results of any completed jobs.
    pub async fn poll_completed(&self) -> Vec<Result<JobResult, JobError>> {
        let mut results = Vec::new();
        let mut jobs = self.active_jobs.lock().await;

        // Collect completed jobs
        while let Some(result) = jobs.try_join_next() {
            match result {
                Ok(job_result) => results.push(job_result),
                Err(e) => results.push(Err(JobError::from(format!("Job panicked: {}", e)))),
            }
        }

        results
    }

    /// Shutdown the worker pool
    ///
    /// Prevents new jobs from being submitted and waits for all active jobs to complete.
    pub async fn shutdown(&self) -> Result<Vec<Result<JobResult, JobError>>, JobError> {
        // Mark as shutting down
        *self.shutting_down.lock().await = true;

        // Wait for all active jobs to complete
        let mut results = Vec::new();
        let mut jobs = self.active_jobs.lock().await;

        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(job_result) => results.push(job_result),
                Err(e) => results.push(Err(JobError::from(format!("Job panicked: {}", e)))),
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    /// Simple test job that sleeps for a specified duration
    struct TestJob {
        id: String,
        duration_ms: u64,
        should_fail: bool,
    }

    impl AsyncJob for TestJob {
        async fn execute(self) -> Result<JobResult, JobError> {
            sleep(Duration::from_millis(self.duration_ms)).await;

            if self.should_fail {
                Err(JobError::from(format!(
                    "Job {} failed as requested",
                    self.id
                )))
            } else {
                Ok(JobResult {
                    job_id: self.id.clone(),
                    success: true,
                    message: Some(format!(
                        "Job {} completed after {}ms",
                        self.id, self.duration_ms
                    )),
                })
            }
        }

        fn job_id(&self) -> String {
            self.id.clone()
        }
    }

    #[tokio::test]
    async fn test_worker_pool_basic_execution() {
        let pool = WorkerPool::new(2);

        // Submit a simple job
        let job = TestJob {
            id: "test-1".to_string(),
            duration_ms: 10,
            should_fail: false,
        };

        pool.try_submit(job).await.unwrap().unwrap();

        // Wait for the job to complete
        sleep(Duration::from_millis(50)).await;

        // Poll for completed jobs
        let completed = pool.poll_completed().await;
        assert_eq!(completed.len(), 1, "Should have 1 completed job");
        assert!(completed[0].is_ok());
        assert!(completed[0].as_ref().unwrap().success);
        assert_eq!(completed[0].as_ref().unwrap().job_id, "test-1");
    }

    #[tokio::test]
    async fn test_worker_pool_concurrent_execution() {
        let pool = WorkerPool::new(2);

        // Submit 4 jobs to a pool with 2 workers
        let mut submitted = 0;
        let mut attempts = 0;
        while submitted < 4 && attempts < 100 {
            let job = TestJob {
                id: format!("job-{}", submitted),
                duration_ms: 50,
                should_fail: false,
            };
            if pool.try_submit(job).await.unwrap().is_some() {
                submitted += 1;
            } else {
                // Wait a bit for a worker to become available
                sleep(Duration::from_millis(10)).await;
            }
            attempts += 1;
        }
        assert_eq!(submitted, 4, "Should have submitted all 4 jobs");

        // Wait for all jobs to complete
        sleep(Duration::from_millis(150)).await;

        // Poll for completed jobs
        let mut all_completed = Vec::new();
        loop {
            let completed = pool.poll_completed().await;
            if completed.is_empty() {
                break;
            }
            all_completed.extend(completed);
        }

        assert_eq!(all_completed.len(), 4, "All 4 jobs should complete");
        for result in &all_completed {
            assert!(result.is_ok());
            assert!(result.as_ref().unwrap().success);
        }
    }

    #[tokio::test]
    async fn test_worker_pool_concurrency_limit() {
        let pool = WorkerPool::new(2);

        // Submit 3 jobs that take 100ms each
        let start = tokio::time::Instant::now();

        let mut submitted = 0;
        let mut attempts = 0;
        while submitted < 3 && attempts < 100 {
            let job = TestJob {
                id: format!("job-{}", submitted),
                duration_ms: 100,
                should_fail: false,
            };
            if pool.try_submit(job).await.unwrap().is_some() {
                submitted += 1;
            } else {
                // Wait a bit for a worker to become available
                sleep(Duration::from_millis(10)).await;
            }
            attempts += 1;
        }
        assert_eq!(submitted, 3, "Should have submitted all 3 jobs");

        // Wait for all jobs to complete
        sleep(Duration::from_millis(250)).await;

        // Poll for all completed jobs
        let mut total_completed = 0;
        loop {
            let completed = pool.poll_completed().await;
            total_completed += completed.len();
            if total_completed >= 3 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }

        let elapsed = start.elapsed();

        // With 2 workers and 3 jobs of 100ms each, it should take ~200ms
        // (2 jobs run in parallel, then the 3rd runs)
        assert_eq!(total_completed, 3, "All 3 jobs should complete");
        assert!(
            elapsed >= Duration::from_millis(150),
            "Should take at least 150ms"
        );
    }

    #[tokio::test]
    async fn test_worker_pool_error_handling() {
        let pool = WorkerPool::new(2);

        // Submit a job that will fail
        let job = TestJob {
            id: "failing-job".to_string(),
            duration_ms: 10,
            should_fail: true,
        };

        pool.try_submit(job).await.unwrap().unwrap();

        // Wait for job to complete
        sleep(Duration::from_millis(50)).await;

        // Poll for completed jobs
        let completed = pool.poll_completed().await;
        assert_eq!(completed.len(), 1, "Should have 1 completed job");

        assert!(completed[0].is_err());
        assert!(
            completed[0]
                .as_ref()
                .unwrap_err()
                .message
                .contains("failed as requested")
        );
    }

    #[tokio::test]
    async fn test_worker_pool_try_submit() {
        let pool = WorkerPool::new(1);

        // Submit a long-running job to occupy the single worker
        let job1 = TestJob {
            id: "long-job".to_string(),
            duration_ms: 200,
            should_fail: false,
        };
        pool.try_submit(job1).await.unwrap().unwrap();

        // Try to submit another job without waiting
        let job2 = TestJob {
            id: "quick-job".to_string(),
            duration_ms: 10,
            should_fail: false,
        };

        let result = pool.try_submit(job2).await.unwrap();
        assert!(
            result.is_none(),
            "Should not be able to submit when pool is full"
        );

        // Wait a bit for the first job to complete
        sleep(Duration::from_millis(250)).await;

        // Now try again
        let job3 = TestJob {
            id: "another-job".to_string(),
            duration_ms: 10,
            should_fail: false,
        };

        let result = pool.try_submit(job3).await.unwrap();
        assert!(
            result.is_some(),
            "Should be able to submit after worker is free"
        );
    }

    #[tokio::test]
    async fn test_worker_pool_shutdown() {
        let pool = WorkerPool::new(2);

        // Submit some jobs
        let mut submitted = 0;
        let mut attempts = 0;
        while submitted < 3 && attempts < 100 {
            let job = TestJob {
                id: format!("job-{}", submitted),
                duration_ms: 50,
                should_fail: false,
            };
            if pool.try_submit(job).await.unwrap().is_some() {
                submitted += 1;
            } else {
                // Wait a bit for a worker to become available
                sleep(Duration::from_millis(10)).await;
            }
            attempts += 1;
        }
        assert_eq!(submitted, 3, "Should have submitted all 3 jobs");

        // Shutdown the pool
        let results = pool.shutdown().await.unwrap();

        // Should get results for all submitted jobs
        assert_eq!(
            results.len(),
            3,
            "Should get results for all 3 submitted jobs"
        );

        // Pool should now reject new submissions
        let job = TestJob {
            id: "late-job".to_string(),
            duration_ms: 10,
            should_fail: false,
        };

        let result = pool.try_submit(job).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.message.contains("shutting down"));
    }

    #[tokio::test]
    async fn test_worker_pool_worker_counts() {
        let pool = WorkerPool::new(3);

        // Initially all workers should be available
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 3);
        assert_eq!(pool.active_workers(), 0);

        // Submit a job
        let job1 = TestJob {
            id: "job-1".to_string(),
            duration_ms: 100,
            should_fail: false,
        };
        pool.try_submit(job1).await.unwrap().unwrap();

        // One worker should be busy
        sleep(Duration::from_millis(10)).await; // Give it a moment to start
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 2);
        assert_eq!(pool.active_workers(), 1);

        // Submit another job
        let job2 = TestJob {
            id: "job-2".to_string(),
            duration_ms: 100,
            should_fail: false,
        };
        pool.try_submit(job2).await.unwrap().unwrap();

        // Two workers should be busy
        sleep(Duration::from_millis(10)).await;
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 1);
        assert_eq!(pool.active_workers(), 2);

        // Wait for jobs to complete
        sleep(Duration::from_millis(120)).await;

        // Poll to clean up completed jobs
        let _ = pool.poll_completed().await;

        // All workers should be available again
        assert_eq!(pool.total_workers(), 3);
        assert_eq!(pool.available_workers(), 3);
        assert_eq!(pool.active_workers(), 0);
    }
}
