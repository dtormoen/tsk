# Task Lifecycle in TSK

## Overview
This document describes the complete lifecycle of a task in TSK, from creation to completion, with a focus on the worker counting bug discovered in the TaskScheduler.

## Task States
Tasks progress through the following states:
1. **Queued** - Task is created and waiting to be executed
2. **Running** - Task is actively being executed by a worker
3. **Complete/Failed** - Task has finished execution (successfully or with error)

## Detailed Task Execution Flow

### 1. Task Scheduling (in TaskScheduler)
The TaskScheduler runs a continuous loop that:

1. **Polls for completed jobs** (`poll_completed()`)
   - Checks WorkerPool for any completed jobs
   - For each completed job:
     - **Decrements active worker count** (this is where the bug fix was attempted)
     - Updates terminal title with new count
     - Removes task from `submitted_tasks` set
     - Logs success/failure message

2. **Checks for new tasks to schedule**
   - If workers are available (`pool.available_workers() > 0`)
   - Finds a queued task that isn't already submitted
   - Updates task status to Running in storage
   - Adds task ID to `submitted_tasks` set
   - **Increments active worker count**
   - Updates terminal title with new count
   - Creates a `TaskJob` and submits it to the WorkerPool

3. **Handles submission failures**
   - If `try_submit()` returns `Ok(None)` or `Err`:
     - Removes task from `submitted_tasks`
     - **Decrements active worker count** (reverting the increment)
     - Reverts task status to Queued

### 2. Job Execution (in WorkerPool)

The WorkerPool manages job execution through:

1. **Job Submission** (`try_submit()`)
   - Attempts to acquire a semaphore permit (without blocking)
   - If successful:
     - Spawns a tokio task to execute the job
     - Returns a `JobHandle` wrapped in `Some`
   - If no permit available:
     - Returns `None`

2. **Job Execution**
   - The spawned task holds the semaphore permit for its duration
   - Executes the job's `execute()` method
   - Permit is automatically released when task completes

3. **Job Completion Polling** (`poll_completed()`)
   - **BUG IDENTIFIED**: Currently tries to get results from `active_jobs` JoinSet
   - **PROBLEM**: Jobs are never added to `active_jobs` in `try_submit()` or `submit()`
   - **RESULT**: Always returns empty vector, so TaskScheduler never sees completed jobs

## The Worker Count Bug

### Root Cause
The WorkerPool's `try_submit()` and `submit()` methods create and spawn job tasks but never add them to the `active_jobs` JoinSet. This means:
1. Jobs execute successfully in spawned tasks
2. Semaphore permits are properly acquired and released
3. But `poll_completed()` can't see any completed jobs
4. TaskScheduler never decrements the worker count
5. Terminal title shows ever-increasing active worker count (e.g., "10/4 workers")

### Previous Fix Attempt
The previous commit attempted to fix this by:
- Moving the worker count decrement to happen for ALL job completions (success or failure)
- Adding decrements when job submission fails

However, this didn't work because the real issue is that `poll_completed()` never returns any completed jobs.

### Required Fix
The WorkerPool needs to:
1. Add spawned job handles to the `active_jobs` JoinSet when submitting jobs
2. This allows `poll_completed()` to actually retrieve completed job results
3. Which triggers the TaskScheduler's worker count decrement logic

## Terminal Title Updates
The terminal title reflects the current state:
- **Idle**: "TSK Server Idle (0/N workers)" when no tasks are running
- **Running**: "TSK Server Running (X/N workers)" where X is active count, N is total

The title should accurately show how many workers are currently executing tasks.