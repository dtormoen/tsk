use crate::context::tsk_env::TskEnv;
use crate::task::Task;
use std::path::PathBuf;
use std::sync::Arc;

/// Trait for task storage abstraction.
///
/// Status mutations use explicit named transition methods that perform targeted
/// SQL updates and return the full updated row, eliminating clone-mutate-replace
/// patterns and ensuring callers always receive the authoritative DB state.
#[async_trait::async_trait]
pub trait TaskStorage: Send + Sync {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get_task(
        &self,
        id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Transition a task to Running status, setting `started_at` to now.
    async fn mark_running(
        &self,
        id: &str,
    ) -> Result<Task, Box<dyn std::error::Error + Send + Sync>>;

    /// Transition a task to Complete status, setting `completed_at` to now and `branch_name`.
    async fn mark_complete(
        &self,
        id: &str,
        branch_name: &str,
    ) -> Result<Task, Box<dyn std::error::Error + Send + Sync>>;

    /// Transition a task to Failed status, setting `completed_at` to now and `error_message`.
    async fn mark_failed(
        &self,
        id: &str,
        error_message: &str,
    ) -> Result<Task, Box<dyn std::error::Error + Send + Sync>>;

    /// Reset a task to Queued status, clearing `started_at`, `completed_at`, and `error_message`.
    async fn reset_to_queued(
        &self,
        id: &str,
    ) -> Result<Task, Box<dyn std::error::Error + Send + Sync>>;

    /// Update a child task's repository fields after copying from its parent.
    /// Sets `copied_repo_path`, `source_commit`, and `source_branch`.
    async fn prepare_child_task(
        &self,
        id: &str,
        copied_repo_path: PathBuf,
        source_commit: &str,
        source_branch: &str,
    ) -> Result<Task, Box<dyn std::error::Error + Send + Sync>>;
}

/// Creates a new SQLite-backed task storage instance for the given environment.
pub(crate) fn get_task_storage(tsk_env: Arc<TskEnv>) -> Arc<dyn TaskStorage> {
    let db_path = tsk_env.tasks_db();
    let storage = crate::sqlite_task_storage::SqliteTaskStorage::new(db_path)
        .expect("Failed to initialize SQLite task storage");
    Arc::new(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::sqlite_task_storage::SqliteTaskStorage;
    use crate::task::{Task, TaskStatus};
    use std::fs;
    use std::path::{Path, PathBuf};

    async fn run_storage_crud_tests(storage: &dyn TaskStorage, data_dir: &Path) {
        let task = Task {
            id: "abcd1234".to_string(),
            repo_root: data_dir.to_path_buf(),
            task_type: "feature".to_string(),
            branch_name: "tsk/test-task".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };

        storage.add_task(task.clone()).await.unwrap();

        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-task");

        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        let updated = storage.mark_running(&task.id).await.unwrap();
        assert_eq!(updated.status, TaskStatus::Running);

        let retrieved = storage.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Running);

        storage.delete_task(&task.id).await.unwrap();
        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_none());

        // Test deleting specific tasks
        let task1 = Task {
            id: "efgh5678".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "task1".to_string(),
            task_type: "feature".to_string(),
            branch_name: "tsk/task1".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        let task2 = Task {
            id: "ijkl9012".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "task2".to_string(),
            task_type: "bug-fix".to_string(),
            status: TaskStatus::Complete,
            branch_name: "tsk/task2".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        let task3 = Task {
            id: "mnop3456".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "task3".to_string(),
            task_type: "refactor".to_string(),
            status: TaskStatus::Failed,
            branch_name: "tsk/task3".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };

        storage.add_task(task1.clone()).await.unwrap();
        storage.add_task(task2.clone()).await.unwrap();
        storage.add_task(task3.clone()).await.unwrap();

        storage.delete_task(&task2.id).await.unwrap();
        storage.delete_task(&task3.id).await.unwrap();

        let remaining_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(remaining_tasks.len(), 1);
        assert_eq!(remaining_tasks[0].status, TaskStatus::Queued);
    }

    #[tokio::test]
    async fn test_sqlite_task_storage() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let db_path = tsk_env.data_dir().join("test_tasks.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();
        run_storage_crud_tests(&storage, tsk_env.data_dir()).await;
    }

    #[tokio::test]
    async fn test_sqlite_round_trip_all_fields() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let db_path = tsk_env.data_dir().join("test_round_trip.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let created_at = chrono::Local::now();
        let started_at = chrono::Utc::now();
        let completed_at = chrono::Utc::now();

        let task = Task {
            id: "round1234".to_string(),
            repo_root: PathBuf::from("/some/repo/root"),
            name: "full-task".to_string(),
            instructions_file: "/tmp/instructions.md".to_string(),
            agent: "codex".to_string(),
            status: TaskStatus::Complete,
            created_at,
            started_at: Some(started_at),
            completed_at: Some(completed_at),
            branch_name: "tsk/feat/full-task/round1234".to_string(),
            error_message: Some("something went wrong".to_string()),
            source_commit: "deadbeef".to_string(),
            source_branch: Some("develop".to_string()),
            stack: "rust".to_string(),
            project: "my-project".to_string(),
            copied_repo_path: Some(PathBuf::from("/copied/repo/path")),
            is_interactive: true,
            parent_ids: vec!["parent5678".to_string()],
            ..Task::test_default()
        };

        storage.add_task(task.clone()).await.unwrap();

        let retrieved = storage.get_task("round1234").await.unwrap().unwrap();

        assert_eq!(retrieved.id, "round1234");
        assert_eq!(retrieved.repo_root, PathBuf::from("/some/repo/root"));
        assert_eq!(retrieved.name, "full-task");
        assert_eq!(retrieved.task_type, "feat");
        assert_eq!(retrieved.instructions_file, "/tmp/instructions.md");
        assert_eq!(retrieved.agent, "codex");
        assert_eq!(retrieved.status, TaskStatus::Complete);
        assert_eq!(
            retrieved.created_at.to_rfc3339(),
            task.created_at.to_rfc3339()
        );
        assert_eq!(
            retrieved.started_at.unwrap().to_rfc3339(),
            started_at.to_rfc3339()
        );
        assert_eq!(
            retrieved.completed_at.unwrap().to_rfc3339(),
            completed_at.to_rfc3339()
        );
        assert_eq!(retrieved.branch_name, "tsk/feat/full-task/round1234");
        assert_eq!(
            retrieved.error_message,
            Some("something went wrong".to_string())
        );
        assert_eq!(retrieved.source_commit, "deadbeef");
        assert_eq!(retrieved.source_branch, Some("develop".to_string()));
        assert_eq!(retrieved.stack, "rust");
        assert_eq!(retrieved.project, "my-project");
        assert_eq!(
            retrieved.copied_repo_path,
            Some(PathBuf::from("/copied/repo/path"))
        );
        assert!(retrieved.is_interactive);
        assert_eq!(retrieved.parent_ids, vec!["parent5678".to_string()]);
    }

    #[tokio::test]
    async fn test_mark_running() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let db_path = tsk_env.data_dir().join("test_mark_running.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task {
            id: "run1234".to_string(),
            repo_root: tsk_env.data_dir().to_path_buf(),
            branch_name: "tsk/fix/run-task/run1234".to_string(),
            copied_repo_path: Some(tsk_env.data_dir().to_path_buf()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let updated = storage.mark_running("run1234").await.unwrap();
        assert_eq!(updated.status, TaskStatus::Running);
        assert!(updated.started_at.is_some());
        assert!(updated.completed_at.is_none());
        assert!(updated.error_message.is_none());
    }

    #[tokio::test]
    async fn test_mark_complete() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let db_path = tsk_env.data_dir().join("test_mark_complete.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task {
            id: "comp1234".to_string(),
            repo_root: tsk_env.data_dir().to_path_buf(),
            branch_name: "tsk/fix/comp-task/comp1234".to_string(),
            copied_repo_path: Some(tsk_env.data_dir().to_path_buf()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        // First mark running, then complete
        storage.mark_running("comp1234").await.unwrap();
        let updated = storage
            .mark_complete("comp1234", "tsk/fix/new-branch/comp1234")
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::Complete);
        assert!(updated.completed_at.is_some());
        assert_eq!(updated.branch_name, "tsk/fix/new-branch/comp1234");
        // started_at should be preserved from mark_running
        assert!(updated.started_at.is_some());
    }

    #[tokio::test]
    async fn test_mark_failed() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let db_path = tsk_env.data_dir().join("test_mark_failed.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task {
            id: "fail1234".to_string(),
            repo_root: tsk_env.data_dir().to_path_buf(),
            branch_name: "tsk/fix/fail-task/fail1234".to_string(),
            copied_repo_path: Some(tsk_env.data_dir().to_path_buf()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        storage.mark_running("fail1234").await.unwrap();
        let updated = storage
            .mark_failed("fail1234", "agent crashed")
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
        assert!(updated.completed_at.is_some());
        assert_eq!(updated.error_message, Some("agent crashed".to_string()));
        // started_at should be preserved from mark_running
        assert!(updated.started_at.is_some());
    }

    #[tokio::test]
    async fn test_reset_to_queued() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let db_path = tsk_env.data_dir().join("test_reset_queued.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task {
            id: "reset1234".to_string(),
            repo_root: tsk_env.data_dir().to_path_buf(),
            branch_name: "tsk/fix/reset-task/reset1234".to_string(),
            copied_repo_path: Some(tsk_env.data_dir().to_path_buf()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        // Move through Running → Failed, then reset
        storage.mark_running("reset1234").await.unwrap();
        storage
            .mark_failed("reset1234", "temporary error")
            .await
            .unwrap();
        let updated = storage.reset_to_queued("reset1234").await.unwrap();
        assert_eq!(updated.status, TaskStatus::Queued);
        assert!(updated.started_at.is_none());
        assert!(updated.completed_at.is_none());
        assert!(updated.error_message.is_none());
    }

    #[tokio::test]
    async fn test_prepare_child_task() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let db_path = tsk_env.data_dir().join("test_prepare_child.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task {
            id: "child1234".to_string(),
            repo_root: tsk_env.data_dir().to_path_buf(),
            branch_name: "tsk/feat/child-task/child1234".to_string(),
            source_commit: "old_commit".to_string(),
            parent_ids: vec!["parent1234".to_string()],
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let repo_path = tsk_env.data_dir().join("copied-repo");
        let updated = storage
            .prepare_child_task(
                "child1234",
                repo_path.clone(),
                "new_commit_sha",
                "tsk/feat/parent-branch/parent1234",
            )
            .await
            .unwrap();
        assert_eq!(updated.copied_repo_path, Some(repo_path));
        assert_eq!(updated.source_commit, "new_commit_sha");
        assert_eq!(
            updated.source_branch,
            Some("tsk/feat/parent-branch/parent1234".to_string())
        );
        // branch_name should remain unchanged (it's the child's own branch)
        assert_eq!(updated.branch_name, "tsk/feat/child-task/child1234");
    }

    #[tokio::test]
    async fn test_migration_from_json() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let data_dir = tsk_env.data_dir();
        let json_path = data_dir.join("tasks.json");
        let bak_path = data_dir.join("tasks.json.bak");
        let db_path = data_dir.join("migration_test.db");

        // Create a tasks.json with known tasks
        let task = Task {
            id: "migrate1234".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "migrate-task".to_string(),
            branch_name: "tsk/feat/migrate-task/migrate1234".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        let tasks = vec![task];
        let json = serde_json::to_string_pretty(&tasks).unwrap();
        fs::write(&json_path, &json).unwrap();

        // Construct SqliteTaskStorage — migration should run automatically
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        // Verify tasks are in SQLite
        let stored_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(stored_tasks.len(), 1);
        assert_eq!(stored_tasks[0].id, "migrate1234");
        assert_eq!(stored_tasks[0].name, "migrate-task");

        // Verify tasks.json was renamed
        assert!(!json_path.exists());
        assert!(bak_path.exists());

        // Clean up
        let _ = fs::remove_file(&bak_path);
    }

    #[tokio::test]
    async fn test_migration_skipped_when_bak_exists() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let data_dir = tsk_env.data_dir();
        let json_path = data_dir.join("tasks.json");
        let bak_path = data_dir.join("tasks.json.bak");
        let db_path = data_dir.join("migration_bak_test.db");

        // Create both tasks.json and tasks.json.bak
        let task = Task {
            id: "skip1234".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "skip-task".to_string(),
            branch_name: "tsk/feat/skip-task/skip1234".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        let json = serde_json::to_string_pretty(&vec![task]).unwrap();
        fs::write(&json_path, &json).unwrap();
        fs::write(&bak_path, "old backup").unwrap();

        let storage = SqliteTaskStorage::new(db_path).unwrap();

        // Migration should NOT have run — DB should be empty
        let stored_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(stored_tasks.len(), 0);

        // tasks.json should still exist (not renamed)
        assert!(json_path.exists());

        // Clean up
        let _ = fs::remove_file(&json_path);
        let _ = fs::remove_file(&bak_path);
    }

    #[tokio::test]
    async fn test_migration_skipped_when_db_has_data() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let data_dir = tsk_env.data_dir();
        let json_path = data_dir.join("tasks.json");
        let bak_path = data_dir.join("tasks.json.bak");
        let db_path = data_dir.join("migration_existing_test.db");

        // First, create a storage and add a task to it
        let storage = SqliteTaskStorage::new(db_path.clone()).unwrap();
        let existing_task = Task {
            id: "existing1234".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "existing-task".to_string(),
            branch_name: "tsk/feat/existing-task/existing1234".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        storage.add_task(existing_task).await.unwrap();
        drop(storage);

        // Now create tasks.json with different tasks
        let json_task = Task {
            id: "json5678".to_string(),
            repo_root: data_dir.to_path_buf(),
            name: "json-task".to_string(),
            branch_name: "tsk/feat/json-task/json5678".to_string(),
            copied_repo_path: Some(data_dir.to_path_buf()),
            ..Task::test_default()
        };
        let json = serde_json::to_string_pretty(&vec![json_task]).unwrap();
        fs::write(&json_path, &json).unwrap();

        // Re-open the storage — migration should NOT run since DB has data
        let storage = SqliteTaskStorage::new(db_path).unwrap();
        let stored_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(stored_tasks.len(), 1);
        assert_eq!(stored_tasks[0].id, "existing1234");

        // tasks.json should still exist (not renamed)
        assert!(json_path.exists());

        // Clean up
        let _ = fs::remove_file(&json_path);
        let _ = fs::remove_file(&bak_path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_writes_no_busy_errors() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let data_dir = tsk_env.data_dir().to_path_buf();

        let db_path = tsk_env.data_dir().join("concurrent_test.db");
        let storage1 = Arc::new(SqliteTaskStorage::new(db_path.clone()).unwrap());
        let storage2 = Arc::new(SqliteTaskStorage::new(db_path).unwrap());

        const TASKS_PER_WRITER: usize = 50;

        let spawn_writer = |storage: Arc<SqliteTaskStorage>, dir: PathBuf, writer_id: usize| {
            tokio::spawn(async move {
                for i in 0..TASKS_PER_WRITER {
                    let task = Task {
                        id: format!("w{writer_id}-t{i}"),
                        repo_root: dir.clone(),
                        name: format!("task-{writer_id}-{i}"),
                        branch_name: format!("tsk/feat/task-{writer_id}-{i}/w{writer_id}-t{i}"),
                        copied_repo_path: None,
                        ..Task::test_default()
                    };
                    storage.add_task(task).await.unwrap();
                }
            })
        };

        let h1 = spawn_writer(Arc::clone(&storage1), data_dir.clone(), 0);
        let h2 = spawn_writer(Arc::clone(&storage2), data_dir, 1);
        tokio::try_join!(h1, h2).unwrap();

        let tasks = storage1.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), TASKS_PER_WRITER * 2);
    }

    #[tokio::test]
    async fn test_migration_handles_invalid_json() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        let data_dir = tsk_env.data_dir();
        let json_path = data_dir.join("tasks.json");
        let bak_path = data_dir.join("tasks.json.bak");
        let db_path = data_dir.join("migration_invalid_test.db");

        // Create tasks.json with invalid content
        fs::write(&json_path, "not valid json {{{").unwrap();

        let storage = SqliteTaskStorage::new(db_path).unwrap();

        // DB should be empty
        let stored_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(stored_tasks.len(), 0);

        // tasks.json should be renamed to .bak even for invalid JSON
        assert!(!json_path.exists());
        assert!(bak_path.exists());

        // Clean up
        let _ = fs::remove_file(&bak_path);
    }
}
