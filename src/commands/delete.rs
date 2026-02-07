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
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;

    async fn setup_test_environment_with_tasks(
        task_ids: Vec<&str>,
    ) -> anyhow::Result<(AppContext, TestGitRepository)> {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories()?;

        // Create a test git repository
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();

        // Add tasks via storage API
        let storage = get_task_storage(tsk_env.clone());
        for (i, task_id) in task_ids.iter().enumerate() {
            let task_dir_path = tsk_env.task_dir(task_id);
            std::fs::create_dir_all(&task_dir_path)?;
            std::fs::write(task_dir_path.join("test.txt"), "test content")?;
            std::fs::write(
                task_dir_path.join("instructions.md"),
                format!("Task {i} instructions"),
            )?;

            let task = crate::task::Task::new(
                task_id.to_string(),
                repo_root.clone(),
                format!("test-task-{i}"),
                "feat".to_string(),
                task_dir_path
                    .join("instructions.md")
                    .to_string_lossy()
                    .to_string(),
                "claude".to_string(),
                format!("tsk/{task_id}"),
                "abc123".to_string(),
                Some("main".to_string()),
                "default".to_string(),
                "default".to_string(),
                chrono::Local::now(),
                Some(task_dir_path),
                false,
                None,
            );
            storage
                .add_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        Ok((ctx, test_repo))
    }

    #[tokio::test]
    async fn test_delete_single_task() {
        let task_id = "test-task-1";
        let (ctx, _test_repo) = setup_test_environment_with_tasks(vec![task_id])
            .await
            .unwrap();
        let tsk_env = ctx.tsk_env();

        let cmd = DeleteCommand {
            task_ids: vec![task_id.to_string()],
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify task directory is deleted
        let task_dir = tsk_env.task_dir(task_id);
        assert!(!task_dir.exists());

        // Verify task is removed from storage
        let storage = get_task_storage(tsk_env);
        let task = storage.get_task(task_id).await.unwrap();
        assert!(task.is_none());
    }

    #[tokio::test]
    async fn test_delete_multiple_tasks() {
        let task_ids = vec!["task-1", "task-2", "task-3"];
        let (ctx, _test_repo) = setup_test_environment_with_tasks(task_ids.clone())
            .await
            .unwrap();
        let tsk_env = ctx.tsk_env();

        let cmd = DeleteCommand {
            task_ids: task_ids.iter().map(|s| s.to_string()).collect(),
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify all task directories are deleted
        for task_id in &task_ids {
            let task_dir = tsk_env.task_dir(task_id);
            assert!(!task_dir.exists());
        }

        // Verify all tasks are removed from storage
        let storage = get_task_storage(tsk_env);
        for task_id in &task_ids {
            let task = storage.get_task(task_id).await.unwrap();
            assert!(task.is_none());
        }
    }

    #[tokio::test]
    async fn test_delete_with_some_failures() {
        let existing_tasks = vec!["task-1", "task-3"];
        let (ctx, _test_repo) = setup_test_environment_with_tasks(existing_tasks.clone())
            .await
            .unwrap();
        let tsk_env = ctx.tsk_env();

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
        for task_id in &existing_tasks {
            let task_dir = tsk_env.task_dir(task_id);
            assert!(!task_dir.exists());
        }
    }

    #[tokio::test]
    async fn test_delete_empty_task_ids() {
        let (ctx, _test_repo) = setup_test_environment_with_tasks(vec![]).await.unwrap();

        let cmd = DeleteCommand { task_ids: vec![] };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No task IDs provided");
    }
}
