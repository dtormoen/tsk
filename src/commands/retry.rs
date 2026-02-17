use super::Command;
use crate::context::AppContext;
use crate::task_manager::{RetryOverrides, TaskManager};
use async_trait::async_trait;
use std::error::Error;

pub struct RetryCommand {
    pub task_ids: Vec<String>,
    pub edit: bool,
    pub name: Option<String>,
    pub agent: Option<String>,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub parent_id: Option<String>,
    pub dind: Option<bool>,
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
            let overrides = RetryOverrides {
                name: self.name.clone(),
                agent: self.agent.clone(),
                stack: self.stack.clone(),
                project: self.project.clone(),
                parent_id: self.parent_id.clone(),
                dind: self.dind,
            };
            match task_manager
                .retry_task(task_id, self.edit, overrides, ctx)
                .await
            {
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
    use crate::task::TaskStatus;
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;

    async fn setup_test_environment_with_completed_tasks(
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

            // Create instructions file
            let instructions_path = task_dir_path.join("instructions.md");
            std::fs::write(
                &instructions_path,
                format!("# Task {i}\n\nInstructions for task {i}"),
            )?;

            let task = crate::task::Task {
                id: task_id.to_string(),
                repo_root: repo_root.clone(),
                name: format!("test-task-{i}"),
                instructions_file: instructions_path.to_string_lossy().to_string(),
                branch_name: format!("tsk/{task_id}"),
                copied_repo_path: Some(task_dir_path),
                status: TaskStatus::Complete,
                started_at: Some(chrono::Utc::now()),
                completed_at: Some(chrono::Utc::now()),
                ..crate::task::Task::test_default()
            };
            storage
                .add_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

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
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new task was created
        let storage = get_task_storage(ctx.tsk_env());
        let all_tasks = storage.list_tasks().await.unwrap();

        // Should have 2 tasks now (original + retry)
        assert_eq!(all_tasks.len(), 2);

        // Find the new task (not the original)
        let new_task = all_tasks.iter().find(|t| t.id != task_id).unwrap();
        assert_eq!(new_task.name, "test-task-0");
        assert_eq!(new_task.status, TaskStatus::Queued);
    }

    #[tokio::test]
    async fn test_retry_with_overrides() {
        let task_id = "test-task-1";
        let (ctx, _test_repo) = setup_test_environment_with_completed_tasks(vec![task_id])
            .await
            .unwrap();

        let cmd = RetryCommand {
            task_ids: vec![task_id.to_string()],
            edit: false,
            name: Some("new-name".to_string()),
            agent: Some("codex".to_string()),
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new task was created with overridden values
        let storage = get_task_storage(ctx.tsk_env());
        let all_tasks = storage.list_tasks().await.unwrap();

        assert_eq!(all_tasks.len(), 2);

        let new_task = all_tasks.iter().find(|t| t.id != task_id).unwrap();
        assert_eq!(new_task.name, "new-name");
        assert_eq!(new_task.agent, "codex");
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
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new tasks were created
        let storage = get_task_storage(ctx.tsk_env());
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
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
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
        let storage = get_task_storage(ctx.tsk_env());
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
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No task IDs provided");
    }

    #[tokio::test]
    async fn test_retry_queued_task_fails() {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let repo_root = test_repo.path().to_path_buf();

        // Create a queued task (should not be retryable)
        let task_id = "queued-task";
        let task_dir_path = tsk_env.task_dir(task_id);
        std::fs::create_dir_all(&task_dir_path).unwrap();

        // Add queued task via storage API
        let task = crate::task::Task {
            id: task_id.to_string(),
            repo_root,
            name: "queued-task".to_string(),
            branch_name: format!("tsk/{task_id}"),
            copied_repo_path: Some(task_dir_path),
            ..crate::task::Task::test_default()
        };
        let storage = get_task_storage(tsk_env);
        storage.add_task(task).await.unwrap();

        let cmd = RetryCommand {
            task_ids: vec![task_id.to_string()],
            edit: false,
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
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
