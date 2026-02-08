use crate::context::tsk_env::TskEnv;
use crate::task::{Task, TaskStatus};
use std::sync::Arc;

// Trait for task storage abstraction
#[async_trait::async_trait]
pub trait TaskStorage: Send + Sync {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get_task(
        &self,
        id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn update_task(&self, task: Task)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn update_task_status(
        &self,
        id: &str,
        status: TaskStatus,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        completed_at: Option<chrono::DateTime<chrono::Utc>>,
        error_message: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// Factory function for getting task storage
pub fn get_task_storage(tsk_env: Arc<TskEnv>) -> Arc<dyn TaskStorage> {
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
    use crate::task::Task;
    use std::fs;
    use std::path::{Path, PathBuf};

    async fn run_storage_crud_tests(storage: &dyn TaskStorage, data_dir: &Path) {
        let task = Task::new(
            "abcd1234".to_string(),
            data_dir.to_path_buf(),
            "test-task".to_string(),
            "feature".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/test-task".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );

        storage.add_task(task.clone()).await.unwrap();

        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-task");

        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        let mut updated_task = task.clone();
        updated_task.status = TaskStatus::Running;
        storage.update_task(updated_task).await.unwrap();

        let retrieved = storage.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Running);

        storage.delete_task(&task.id).await.unwrap();
        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_none());

        // Test deleting specific tasks
        let task1 = Task::new(
            "efgh5678".to_string(),
            data_dir.to_path_buf(),
            "task1".to_string(),
            "feature".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/task1".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
        let mut task2 = Task::new(
            "ijkl9012".to_string(),
            data_dir.to_path_buf(),
            "task2".to_string(),
            "bug-fix".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/task2".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
        task2.status = TaskStatus::Complete;
        let mut task3 = Task::new(
            "mnop3456".to_string(),
            data_dir.to_path_buf(),
            "task3".to_string(),
            "refactor".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/task3".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
        task3.status = TaskStatus::Failed;

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

        let mut task = Task::new(
            "round1234".to_string(),
            PathBuf::from("/some/repo/root"),
            "full-task".to_string(),
            "feat".to_string(),
            "/tmp/instructions.md".to_string(),
            "codex".to_string(),
            "tsk/feat/full-task/round1234".to_string(),
            "deadbeef".to_string(),
            Some("develop".to_string()),
            "rust".to_string(),
            "my-project".to_string(),
            created_at,
            Some(PathBuf::from("/copied/repo/path")),
            true,
            vec!["parent5678".to_string()],
        );
        task.status = TaskStatus::Complete;
        task.started_at = Some(started_at);
        task.completed_at = Some(completed_at);
        task.error_message = Some("something went wrong".to_string());

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
    async fn test_sqlite_update_task_status() {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        let db_path = tsk_env.data_dir().join("test_status_update.db");
        let storage = SqliteTaskStorage::new(db_path).unwrap();

        let task = Task::new(
            "status1234".to_string(),
            tsk_env.data_dir().to_path_buf(),
            "status-task".to_string(),
            "fix".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/fix/status-task/status1234".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(tsk_env.data_dir().to_path_buf()),
            false,
            vec![],
        );

        storage.add_task(task).await.unwrap();

        // Update to Running with started_at
        let started = chrono::Utc::now();
        storage
            .update_task_status("status1234", TaskStatus::Running, Some(started), None, None)
            .await
            .unwrap();

        let retrieved = storage.get_task("status1234").await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Running);
        assert_eq!(
            retrieved.started_at.unwrap().to_rfc3339(),
            started.to_rfc3339()
        );
        assert!(retrieved.completed_at.is_none());
        assert!(retrieved.error_message.is_none());

        // Update to Complete with completed_at
        let completed = chrono::Utc::now();
        storage
            .update_task_status(
                "status1234",
                TaskStatus::Complete,
                None,
                Some(completed),
                None,
            )
            .await
            .unwrap();

        let retrieved = storage.get_task("status1234").await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Complete);
        // started_at should still be preserved from the previous update
        assert_eq!(
            retrieved.started_at.unwrap().to_rfc3339(),
            started.to_rfc3339()
        );
        assert_eq!(
            retrieved.completed_at.unwrap().to_rfc3339(),
            completed.to_rfc3339()
        );

        // Update to Failed with error_message
        storage
            .update_task_status(
                "status1234",
                TaskStatus::Failed,
                None,
                None,
                Some("agent crashed".to_string()),
            )
            .await
            .unwrap();

        let retrieved = storage.get_task("status1234").await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Failed);
        assert_eq!(retrieved.error_message, Some("agent crashed".to_string()));
        // started_at and completed_at should still be preserved
        assert!(retrieved.started_at.is_some());
        assert!(retrieved.completed_at.is_some());
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
        let task = Task::new(
            "migrate1234".to_string(),
            data_dir.to_path_buf(),
            "migrate-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/feat/migrate-task/migrate1234".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "rust".to_string(),
            "test-project".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
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
        let task = Task::new(
            "skip1234".to_string(),
            data_dir.to_path_buf(),
            "skip-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/feat/skip-task/skip1234".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
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
        let existing_task = Task::new(
            "existing1234".to_string(),
            data_dir.to_path_buf(),
            "existing-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/feat/existing-task/existing1234".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
        storage.add_task(existing_task).await.unwrap();
        drop(storage);

        // Now create tasks.json with different tasks
        let json_task = Task::new(
            "json5678".to_string(),
            data_dir.to_path_buf(),
            "json-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/feat/json-task/json5678".to_string(),
            "abc123".to_string(),
            Some("main".to_string()),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            Some(data_dir.to_path_buf()),
            false,
            vec![],
        );
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
                    let task = Task::new(
                        format!("w{writer_id}-t{i}"),
                        dir.clone(),
                        format!("task-{writer_id}-{i}"),
                        "feat".to_string(),
                        "instructions.md".to_string(),
                        "claude".to_string(),
                        format!("tsk/feat/task-{writer_id}-{i}/w{writer_id}-t{i}"),
                        "abc123".to_string(),
                        Some("main".to_string()),
                        "default".to_string(),
                        "default".to_string(),
                        chrono::Local::now(),
                        None,
                        false,
                        vec![],
                    );
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
