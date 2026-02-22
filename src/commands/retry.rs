use super::Command;
use crate::context::AppContext;
use crate::task::Task;
use crate::task_manager::{RetryOverrides, TaskManager};
use async_trait::async_trait;
use std::collections::HashMap;
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
    pub no_children: bool,
}

fn confirm_retry_children(task_id: &str, descendants: &[Task]) -> bool {
    use is_terminal::IsTerminal;
    use std::io::Write;

    let names: Vec<&str> = descendants.iter().map(|t| t.name.as_str()).collect();
    println!(
        "Task {task_id} has {} child task(s): {}",
        descendants.len(),
        names.join(", ")
    );

    // Non-TTY: default to yes
    if !std::io::stdin().is_terminal() {
        println!("Retrying children (non-interactive mode)");
        return true;
    }

    print!("Retry children too? [Y/n] ");
    let _ = std::io::stdout().flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return true; // default yes on read error
    }

    let trimmed = input.trim().to_lowercase();
    trimmed.is_empty() || trimmed == "y" || trimmed == "yes"
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

                    // Check for descendant tasks to retry
                    let descendants = task_manager.find_descendant_tasks(task_id).await?;
                    if !descendants.is_empty()
                        && !self.no_children
                        && confirm_retry_children(task_id, &descendants)
                    {
                        // Map old IDs to new IDs for parent chain reconstruction
                        let mut id_map = HashMap::new();
                        id_map.insert(task_id.to_string(), new_task_id.clone());

                        for descendant in &descendants {
                            // Find the new parent ID by looking up the old parent
                            let new_parent_id = descendant
                                .parent_ids
                                .first()
                                .and_then(|old_parent| id_map.get(old_parent))
                                .cloned();

                            let child_overrides = RetryOverrides {
                                parent_id: new_parent_id.clone(),
                                ..Default::default()
                            };

                            let parent_display = new_parent_id.as_deref().unwrap_or("none");
                            println!(
                                "Retrying child task: {} (parent: {parent_display})",
                                descendant.id
                            );

                            match task_manager
                                .retry_task(&descendant.id, false, child_overrides, ctx)
                                .await
                            {
                                Ok(new_child_id) => {
                                    println!(
                                        "Task '{}' retried successfully. New task ID: {new_child_id}",
                                        descendant.id
                                    );
                                    id_map.insert(descendant.id.clone(), new_child_id);
                                    successful_retries += 1;
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Failed to retry child task '{}': {e}",
                                        descendant.id
                                    );
                                    failed_retries += 1;
                                }
                            }
                        }
                    }
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
        let storage = ctx.task_storage();
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
            no_children: true,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new task was created
        let storage = ctx.task_storage();
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
            no_children: true,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new task was created with overridden values
        let storage = ctx.task_storage();
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
            no_children: true,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Verify new tasks were created
        let storage = ctx.task_storage();
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
            no_children: true,
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
        let storage = ctx.task_storage();
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
            no_children: true,
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
        let storage = ctx.task_storage();
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
            no_children: true,
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

    #[tokio::test]
    async fn test_retry_with_no_children_flag() {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let repo_root = test_repo.path().to_path_buf();

        let parent_id = "parent-retry-001";
        let child_id = "child-retry-001";

        let storage = ctx.task_storage();

        // Create parent task (failed, retryable)
        let parent_dir = tsk_env.task_dir(parent_id);
        std::fs::create_dir_all(&parent_dir).unwrap();
        let parent_instructions = parent_dir.join("instructions.md");
        std::fs::write(&parent_instructions, "Parent task instructions").unwrap();

        storage
            .add_task(crate::task::Task {
                id: parent_id.to_string(),
                repo_root: repo_root.clone(),
                name: "parent-task".to_string(),
                instructions_file: parent_instructions.to_string_lossy().to_string(),
                branch_name: format!("tsk/{parent_id}"),
                status: TaskStatus::Failed,
                copied_repo_path: Some(parent_dir),
                started_at: Some(chrono::Utc::now()),
                ..crate::task::Task::test_default()
            })
            .await
            .unwrap();

        // Create child task (failed, with parent)
        let child_dir = tsk_env.task_dir(child_id);
        std::fs::create_dir_all(&child_dir).unwrap();
        let child_instructions = child_dir.join("instructions.md");
        std::fs::write(&child_instructions, "Child task instructions").unwrap();

        storage
            .add_task(crate::task::Task {
                id: child_id.to_string(),
                repo_root: repo_root.clone(),
                name: "child-task".to_string(),
                instructions_file: child_instructions.to_string_lossy().to_string(),
                branch_name: format!("tsk/{child_id}"),
                status: TaskStatus::Failed,
                copied_repo_path: Some(child_dir),
                parent_ids: vec![parent_id.to_string()],
                started_at: Some(chrono::Utc::now()),
                ..crate::task::Task::test_default()
            })
            .await
            .unwrap();

        // Retry with --no-children: only parent should be retried
        let cmd = RetryCommand {
            task_ids: vec![parent_id.to_string()],
            edit: false,
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
            no_children: true,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Should have 3 tasks: original parent, original child, retried parent
        let all_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(all_tasks.len(), 3);

        let queued_tasks: Vec<_> = all_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .collect();
        assert_eq!(queued_tasks.len(), 1, "Only the parent should be retried");
    }

    #[tokio::test]
    async fn test_retry_with_children() {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories().unwrap();

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();
        let repo_root = test_repo.path().to_path_buf();

        let parent_id = "parent-chain-001";
        let child_id = "child-chain-001";

        let storage = ctx.task_storage();

        // Create parent task (failed)
        let parent_dir = tsk_env.task_dir(parent_id);
        std::fs::create_dir_all(&parent_dir).unwrap();
        let parent_instructions = parent_dir.join("instructions.md");
        std::fs::write(&parent_instructions, "Parent instructions").unwrap();

        storage
            .add_task(crate::task::Task {
                id: parent_id.to_string(),
                repo_root: repo_root.clone(),
                name: "parent-task".to_string(),
                instructions_file: parent_instructions.to_string_lossy().to_string(),
                branch_name: format!("tsk/{parent_id}"),
                status: TaskStatus::Failed,
                copied_repo_path: Some(parent_dir),
                started_at: Some(chrono::Utc::now()),
                ..crate::task::Task::test_default()
            })
            .await
            .unwrap();

        // Create child task (failed, chained to parent)
        let child_dir = tsk_env.task_dir(child_id);
        std::fs::create_dir_all(&child_dir).unwrap();
        let child_instructions = child_dir.join("instructions.md");
        std::fs::write(&child_instructions, "Child instructions").unwrap();

        storage
            .add_task(crate::task::Task {
                id: child_id.to_string(),
                repo_root: repo_root.clone(),
                name: "child-task".to_string(),
                instructions_file: child_instructions.to_string_lossy().to_string(),
                branch_name: format!("tsk/{child_id}"),
                status: TaskStatus::Failed,
                copied_repo_path: Some(child_dir),
                parent_ids: vec![parent_id.to_string()],
                started_at: Some(chrono::Utc::now()),
                ..crate::task::Task::test_default()
            })
            .await
            .unwrap();

        // Retry parent with no_children=false (non-TTY defaults to yes, so children are retried)
        let cmd = RetryCommand {
            task_ids: vec![parent_id.to_string()],
            edit: false,
            name: None,
            agent: None,
            stack: None,
            project: None,
            parent_id: None,
            dind: None,
            no_children: false,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        // Should have 4 tasks: original parent, original child, retried parent, retried child
        let all_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(all_tasks.len(), 4);

        let new_tasks: Vec<_> = all_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .collect();
        assert_eq!(
            new_tasks.len(),
            2,
            "Both parent and child should be retried"
        );

        // Verify the retried child has the retried parent as its parent
        let new_parent = new_tasks
            .iter()
            .find(|t| t.parent_ids.is_empty())
            .expect("Retried parent should have no parent_ids");
        let new_child = new_tasks
            .iter()
            .find(|t| !t.parent_ids.is_empty())
            .expect("Retried child should have parent_ids");
        assert_eq!(
            new_child.parent_ids,
            vec![new_parent.id.clone()],
            "Retried child should point to the retried parent"
        );
    }
}
