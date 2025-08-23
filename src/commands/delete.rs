use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct DeleteCommand {
    pub task_ids: Vec<String>,
}

#[async_trait]
impl Command for DeleteCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.task_ids.is_empty() {
            return Err("No task IDs provided".into());
        }

        let task_manager = TaskManager::new(ctx)?;
        let mut successful_deletes = 0;
        let mut failed_deletes = 0;

        for task_id in &self.task_ids {
            println!("Deleting task: {task_id}");
            match task_manager.delete_task(task_id).await {
                Ok(()) => {
                    println!("Task '{task_id}' deleted successfully");
                    successful_deletes += 1;
                }
                Err(e) => {
                    eprintln!("Failed to delete task '{task_id}': {e}");
                    failed_deletes += 1;
                }
            }
        }

        if failed_deletes > 0 {
            if successful_deletes > 0 {
                println!(
                    "\nSummary: {successful_deletes} tasks deleted successfully, {failed_deletes} failed"
                );
            }
            return Err(format!("{failed_deletes} task(s) failed to delete").into());
        }

        if self.task_ids.len() > 1 {
            println!("\nAll {} tasks deleted successfully", self.task_ids.len());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::DefaultFileSystem;
    use crate::storage::{XdgConfig, XdgDirectories};
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn setup_test_environment_with_tasks(
        task_ids: Vec<&str>,
    ) -> anyhow::Result<(AppContext, TestGitRepository, Arc<XdgDirectories>, TempDir)> {
        // Create temporary directory for XDG
        let temp_dir = TempDir::new()?;

        // Create XdgDirectories instance
        let config = XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = Arc::new(XdgDirectories::new(Some(config))?);
        xdg.ensure_directories()?;

        // Create a test git repository
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create tasks
        let mut tasks_json = Vec::new();
        for (i, task_id) in task_ids.iter().enumerate() {
            let task_dir_path = xdg.task_dir(task_id, &repo_hash);
            std::fs::create_dir_all(&task_dir_path)?;
            std::fs::write(task_dir_path.join("test.txt"), "test content")?;
            std::fs::write(
                task_dir_path.join("instructions.md"),
                format!("Task {i} instructions"),
            )?;

            let instructions_path = task_dir_path.join("instructions.md");
            tasks_json.push(format!(
                r#"{{"id":"{}","repo_root":"{}","name":"test-task-{}","task_type":"feat","instructions_file":"{}","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","tech_stack":"default","project":"default","copied_repo_path":"{}"}}"#,
                task_id,
                repo_root.to_string_lossy(),
                i,
                instructions_path.to_string_lossy(),
                task_id,
                task_dir_path.to_string_lossy()
            ));
        }

        // Write tasks.json
        let tasks_json_content = format!("[{}]", tasks_json.join(","));
        let tasks_file_path = xdg.tasks_file();
        if let Some(parent) = tasks_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&tasks_file_path, &tasks_json_content)?;

        // Create AppContext
        let ctx = AppContext::builder()
            .with_xdg_directories(xdg.clone())
            .build();

        Ok((ctx, test_repo, xdg, temp_dir))
    }

    #[tokio::test]
    async fn test_delete_single_task() {
        let task_id = "test-task-1";
        let (ctx, _test_repo, xdg, _temp_dir) = setup_test_environment_with_tasks(vec![task_id])
            .await
            .unwrap();

        let cmd = DeleteCommand {
            task_ids: vec![task_id.to_string()],
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify task directory is deleted
        let repo_hash = crate::storage::get_repo_hash(_test_repo.path());
        let task_dir = xdg.task_dir(task_id, &repo_hash);
        assert!(!task_dir.exists());

        // Verify task is removed from storage
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(xdg.clone(), file_system);
        let task = storage.get_task(task_id).await.unwrap();
        assert!(task.is_none());
    }

    #[tokio::test]
    async fn test_delete_multiple_tasks() {
        let task_ids = vec!["task-1", "task-2", "task-3"];
        let (ctx, _test_repo, xdg, _temp_dir) = setup_test_environment_with_tasks(task_ids.clone())
            .await
            .unwrap();

        let cmd = DeleteCommand {
            task_ids: task_ids.iter().map(|s| s.to_string()).collect(),
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify all task directories are deleted
        let repo_hash = crate::storage::get_repo_hash(_test_repo.path());
        for task_id in &task_ids {
            let task_dir = xdg.task_dir(task_id, &repo_hash);
            assert!(!task_dir.exists());
        }

        // Verify all tasks are removed from storage
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(xdg.clone(), file_system);
        for task_id in &task_ids {
            let task = storage.get_task(task_id).await.unwrap();
            assert!(task.is_none());
        }
    }

    #[tokio::test]
    async fn test_delete_with_some_failures() {
        let existing_tasks = vec!["task-1", "task-3"];
        let (ctx, _test_repo, xdg, _temp_dir) =
            setup_test_environment_with_tasks(existing_tasks.clone())
                .await
                .unwrap();

        // Try to delete both existing and non-existing tasks
        let cmd = DeleteCommand {
            task_ids: vec![
                "task-1".to_string(),
                "non-existent".to_string(),
                "task-3".to_string(),
            ],
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 task(s) failed to delete")
        );

        // Verify existing tasks were still deleted
        let repo_hash = crate::storage::get_repo_hash(_test_repo.path());
        for task_id in &existing_tasks {
            let task_dir = xdg.task_dir(task_id, &repo_hash);
            assert!(!task_dir.exists());
        }
    }

    #[tokio::test]
    async fn test_delete_empty_task_ids() {
        let (ctx, _test_repo, _xdg, _temp_dir) =
            setup_test_environment_with_tasks(vec![]).await.unwrap();

        let cmd = DeleteCommand { task_ids: vec![] };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No task IDs provided");
    }
}
