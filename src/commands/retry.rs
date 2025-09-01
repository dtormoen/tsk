use super::Command;
use crate::context::AppContext;
use crate::task_manager::TaskManager;
use async_trait::async_trait;
use std::error::Error;

pub struct RetryCommand {
    pub task_ids: Vec<String>,
    pub edit: bool,
}

#[async_trait]
impl Command for RetryCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.task_ids.is_empty() {
            return Err("No task IDs provided".into());
        }

        let task_manager = TaskManager::new(ctx)?;
        let mut successful_retries = 0;
        let mut failed_retries = 0;

        for task_id in &self.task_ids {
            println!("Retrying task: {task_id}");
            match task_manager.retry_task(task_id, self.edit, ctx).await {
                Ok(new_task_id) => {
                    println!("Task '{task_id}' retried successfully. New task ID: {new_task_id}");
                    successful_retries += 1;
                }
                Err(e) => {
                    eprintln!("Failed to retry task '{task_id}': {e}");
                    failed_retries += 1;
                }
            }
        }

        if failed_retries > 0 {
            if successful_retries > 0 {
                println!(
                    "\nSummary: {successful_retries} tasks retried successfully, {failed_retries} failed"
                );
            }
            return Err(format!("{failed_retries} task(s) failed to retry").into());
        }

        if self.task_ids.len() > 1 {
            println!("\nAll {} tasks retried successfully", self.task_ids.len());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::DefaultFileSystem;
    use crate::task::TaskStatus;
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;
    use std::sync::Arc;

    async fn setup_test_environment_with_completed_tasks(
        task_ids: Vec<&str>,
    ) -> anyhow::Result<(AppContext, TestGitRepository)> {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        config.ensure_directories()?;

        // Create a test git repository
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create tasks
        let mut tasks_json = Vec::new();
        for (i, task_id) in task_ids.iter().enumerate() {
            let task_dir_path = config.task_dir(task_id, &repo_hash);
            std::fs::create_dir_all(&task_dir_path)?;

            // Create instructions file
            let instructions_path = task_dir_path.join("instructions.md");
            std::fs::write(
                &instructions_path,
                format!("# Task {i}\n\nInstructions for task {i}"),
            )?;

            tasks_json.push(format!(
                r#"{{"id":"{}","repo_root":"{}","name":"test-task-{}","task_type":"feat","instructions_file":"{}","agent":"claude-code","timeout":30,"status":"COMPLETE","created_at":"2024-01-01T00:00:00Z","started_at":"2024-01-01T00:30:00Z","completed_at":"2024-01-01T01:00:00Z","branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","stack":"default","project":"default","copied_repo_path":"{}"}}"#,
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
        std::fs::write(config.tasks_file(), tasks_json_content)?;

        Ok((ctx, test_repo))
    }

    #[tokio::test]
    async fn test_retry_single_task() {
        let task_id = "test-task-1";
        let (ctx, _test_repo) = setup_test_environment_with_completed_tasks(vec![task_id])
            .await
            .unwrap();

        let cmd = RetryCommand {
            task_ids: vec![task_id.to_string()],
            edit: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new task was created
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(ctx.tsk_config(), file_system);
        let all_tasks = storage.list_tasks().await.unwrap();

        // Should have 2 tasks now (original + retry)
        assert_eq!(all_tasks.len(), 2);

        // Find the new task (not the original)
        let new_task = all_tasks.iter().find(|t| t.id != task_id).unwrap();
        assert_eq!(new_task.name, "test-task-0");
        assert_eq!(new_task.status, TaskStatus::Queued);
    }

    #[tokio::test]
    async fn test_retry_multiple_tasks() {
        let task_ids = vec!["task-1", "task-2", "task-3"];
        let (ctx, _test_repo) = setup_test_environment_with_completed_tasks(task_ids.clone())
            .await
            .unwrap();

        let cmd = RetryCommand {
            task_ids: task_ids.iter().map(|s| s.to_string()).collect(),
            edit: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new tasks were created
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(ctx.tsk_config(), file_system);
        let all_tasks = storage.list_tasks().await.unwrap();

        // Should have 6 tasks now (3 originals + 3 retries)
        assert_eq!(all_tasks.len(), 6);

        // Count queued tasks (the new ones)
        let queued_count = all_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .count();
        assert_eq!(queued_count, 3);
    }

    #[tokio::test]
    async fn test_retry_with_some_failures() {
        let existing_tasks = vec!["task-1", "task-3"];
        let (ctx, _test_repo) = setup_test_environment_with_completed_tasks(existing_tasks.clone())
            .await
            .unwrap();

        // Try to retry both existing and non-existing tasks
        let cmd = RetryCommand {
            task_ids: vec![
                "task-1".to_string(),
                "non-existent".to_string(),
                "task-3".to_string(),
            ],
            edit: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 task(s) failed to retry")
        );

        // Verify existing tasks were still retried
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(ctx.tsk_config(), file_system);
        let all_tasks = storage.list_tasks().await.unwrap();

        // Should have 4 tasks (2 originals + 2 retries)
        assert_eq!(all_tasks.len(), 4);

        // Count queued tasks (the new ones)
        let queued_count = all_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .count();
        assert_eq!(queued_count, 2);
    }

    #[tokio::test]
    async fn test_retry_empty_task_ids() {
        let (ctx, _test_repo) = setup_test_environment_with_completed_tasks(vec![])
            .await
            .unwrap();

        let cmd = RetryCommand {
            task_ids: vec![],
            edit: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No task IDs provided");
    }

    #[tokio::test]
    async fn test_retry_queued_task_fails() {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        config.ensure_directories().unwrap();

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create a queued task (should not be retryable)
        let task_id = "queued-task";
        let task_dir_path = config.task_dir(task_id, &repo_hash);
        std::fs::create_dir_all(&task_dir_path).unwrap();

        let task_json = format!(
            r#"[{{"id":"{}","repo_root":"{}","name":"queued-task","task_type":"feat","instructions_file":"instructions.md","agent":"claude-code","timeout":30,"status":"QUEUED","created_at":"2024-01-01T00:00:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/{}","error_message":null,"source_commit":"abc123","stack":"default","project":"default","copied_repo_path":"{}"}}]"#,
            task_id,
            repo_root.to_string_lossy(),
            task_id,
            task_dir_path.to_string_lossy()
        );
        std::fs::write(config.tasks_file(), task_json).unwrap();

        let cmd = RetryCommand {
            task_ids: vec![task_id.to_string()],
            edit: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 task(s) failed to retry")
        );
    }
}
