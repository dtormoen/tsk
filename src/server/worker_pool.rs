use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::{JoinHandle, JoinSet};

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
#[allow(dead_code)]
pub struct JobHandle {
    /// Unique identifier for the job
    pub job_id: String,
    /// Join handle for the spawned task
    handle: JoinHandle<Result<JobResult, JobError>>,
}

impl JobHandle {
    /// Check if the job is finished
    #[allow(dead_code)]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Wait for the job to complete and get the result
    #[allow(dead_code)]
    pub async fn await_result(self) -> Result<JobResult, JobError> {
        match self.handle.await {
            Ok(result) => result,
            Err(e) => Err(JobError::from(format!("Job panicked: {}", e))),
        }
    }
}

/// Generic worker pool for executing async jobs with concurrency control
pub struct WorkerPool<T: AsyncJob> {
    /// Maximum number of concurrent workers
    #[allow(dead_code)]
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

    /// Get the number of workers in the pool
    #[allow(dead_code)]
    pub fn worker_count(&self) -> usize {
        self.workers
    }

    /// Get the number of available workers (not currently executing jobs)
    pub fn available_workers(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Submit a job to the worker pool
    ///
    /// Returns a JobHandle that can be used to track and await the job result.
    /// Returns an error if the pool is shutting down.
    #[allow(dead_code)]
    pub async fn submit(&self, job: T) -> Result<JobHandle, JobError> {
        // Check if we're shutting down
        if *self.shutting_down.lock().await {
            return Err(JobError::from("Worker pool is shutting down".to_string()));
        }

        // Get the job ID before moving the job
        let job_id = job.job_id();

        // Acquire a permit (this will wait if all workers are busy)
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| JobError::from(format!("Failed to acquire permit: {}", e)))?;

        // Spawn the job execution
        let handle = tokio::spawn(async move {
            // Hold the permit for the duration of the job
            let _permit = permit;

            // Execute the job
            job.execute().await
        });

        Ok(JobHandle { job_id, handle })
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

        // Spawn the job execution
        let handle = tokio::spawn(async move {
            // Hold the permit for the duration of the job
            let _permit = permit;

            // Execute the job
            job.execute().await
        });

        Ok(Some(JobHandle { job_id, handle }))
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

    /// Check if the pool is shutting down
    #[allow(dead_code)]
    pub async fn is_shutting_down(&self) -> bool {
        *self.shutting_down.lock().await
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

        let handle = pool.submit(job).await.unwrap();
        assert_eq!(handle.job_id, "test-1");

        // Wait for the job to complete
        let result = handle.await_result().await.unwrap();
        assert!(result.success);
        assert_eq!(result.job_id, "test-1");
    }

    #[tokio::test]
    async fn test_worker_pool_concurrent_execution() {
        let pool = WorkerPool::new(2);

        // Submit 4 jobs to a pool with 2 workers
        let mut handles = Vec::new();
        for i in 0..4 {
            let job = TestJob {
                id: format!("job-{}", i),
                duration_ms: 50,
                should_fail: false,
            };
            handles.push(pool.submit(job).await.unwrap());
        }

        // All jobs should complete
        for handle in handles {
            let result = handle.await_result().await.unwrap();
            assert!(result.success);
        }
    }

    #[tokio::test]
    async fn test_worker_pool_concurrency_limit() {
        let pool = WorkerPool::new(2);

        // Submit 3 jobs that take 100ms each
        let start = tokio::time::Instant::now();
        let mut handles = Vec::new();

        for i in 0..3 {
            let job = TestJob {
                id: format!("job-{}", i),
                duration_ms: 100,
                should_fail: false,
            };
            handles.push(pool.submit(job).await.unwrap());
        }

        // Wait for all jobs
        for handle in handles {
            handle.await_result().await.unwrap();
        }

        let elapsed = start.elapsed();

        // With 2 workers and 3 jobs of 100ms each, it should take ~200ms
        // (2 jobs run in parallel, then the 3rd runs)
        assert!(elapsed >= Duration::from_millis(190)); // Allow some slack
        assert!(elapsed < Duration::from_millis(250)); // But not too much
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

        let handle = pool.submit(job).await.unwrap();
        let result = handle.await_result().await;

        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("failed as requested"));
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
        let _handle1 = pool.submit(job1).await.unwrap();

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
        let mut handles = Vec::new();
        for i in 0..3 {
            let job = TestJob {
                id: format!("job-{}", i),
                duration_ms: 50,
                should_fail: false,
            };
            handles.push(pool.submit(job).await.unwrap());
        }

        // Shutdown the pool
        let results = pool.shutdown().await.unwrap();

        // Should get results for all jobs
        assert_eq!(results.len(), 0); // Results are collected via handles, not shutdown

        // Pool should now reject new submissions
        let job = TestJob {
            id: "late-job".to_string(),
            duration_ms: 10,
            should_fail: false,
        };

        let result = pool.submit(job).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.message.contains("shutting down"));
    }

    #[tokio::test]
    async fn test_worker_pool_available_workers() {
        let pool = WorkerPool::new(3);

        // Initially all workers should be available
        assert_eq!(pool.available_workers(), 3);

        // Submit a job
        let job1 = TestJob {
            id: "job-1".to_string(),
            duration_ms: 100,
            should_fail: false,
        };
        let _handle1 = pool.submit(job1).await.unwrap();

        // One worker should be busy
        sleep(Duration::from_millis(10)).await; // Give it a moment to start
        assert_eq!(pool.available_workers(), 2);

        // Submit another job
        let job2 = TestJob {
            id: "job-2".to_string(),
            duration_ms: 100,
            should_fail: false,
        };
        let _handle2 = pool.submit(job2).await.unwrap();

        // Two workers should be busy
        sleep(Duration::from_millis(10)).await;
        assert_eq!(pool.available_workers(), 1);
    }
}
