use super::Command;
use crate::context::AppContext;
use crate::display::{colorize_status, format_duration, print_columns, status_color};
use crate::task::TaskStatus;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use chrono::Utc;
use is_terminal::IsTerminal;
use std::error::Error;

pub struct ListCommand;

#[async_trait]
impl Command for ListCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let storage = get_task_storage(ctx.tsk_env());
        let tasks = storage
            .list_tasks()
            .await
            .map_err(|e| e as Box<dyn Error>)?;

        if tasks.is_empty() {
            println!("No tasks in queue");
        } else {
            let styled = std::io::stdout().is_terminal();

            let rows: Vec<Vec<String>> = tasks
                .iter()
                .map(|task| {
                    let status = match &task.status {
                        TaskStatus::Queued => {
                            if !task.parent_ids.is_empty() && task.copied_repo_path.is_none() {
                                "WAITING".to_string()
                            } else {
                                "QUEUED".to_string()
                            }
                        }
                        TaskStatus::Running => "RUNNING".to_string(),
                        TaskStatus::Failed => "FAILED".to_string(),
                        TaskStatus::Complete => "COMPLETE".to_string(),
                    };

                    let duration = match (&task.status, &task.started_at, &task.completed_at) {
                        (TaskStatus::Complete | TaskStatus::Failed, Some(start), Some(end)) => {
                            let secs = (*end - *start).num_seconds();
                            format_duration(secs)
                        }
                        (TaskStatus::Running, Some(start), _) => {
                            let secs = (Utc::now() - *start).num_seconds();
                            format_duration(secs)
                        }
                        _ => "-".to_string(),
                    };

                    vec![
                        task.id.clone(),
                        task.name.clone(),
                        task.task_type.clone(),
                        colorize_status(&status, styled),
                        duration,
                        if task.parent_ids.is_empty() {
                            "-".to_string()
                        } else {
                            task.parent_ids.join(",")
                        },
                        task.agent.clone(),
                        task.branch_name.clone(),
                        task.created_at.format("%Y-%m-%d %H:%M").to_string(),
                    ]
                })
                .collect();

            let headers = [
                "ID", "Name", "Type", "Status", "Duration", "Parent", "Agent", "Branch", "Created",
            ];
            print_columns(&headers, &rows);

            // Print summary
            let waiting = tasks
                .iter()
                .filter(|t| {
                    t.status == TaskStatus::Queued
                        && !t.parent_ids.is_empty()
                        && t.copied_repo_path.is_none()
                })
                .count();
            let queued = tasks
                .iter()
                .filter(|t| {
                    t.status == TaskStatus::Queued
                        && (t.parent_ids.is_empty() || t.copied_repo_path.is_some())
                })
                .count();
            let running = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Running)
                .count();
            let complete = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Complete)
                .count();
            let failed = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Failed)
                .count();

            let cs = |count: usize, label: &str| -> String {
                if styled
                    && count > 0
                    && let Some(code) = status_color(label)
                {
                    return format!("{code}{count} {label}\x1b[0m");
                }
                format!("{count} {label}")
            };
            println!(
                "\nSummary: {}, {}, {}, {}, {}",
                cs(queued, "queued"),
                cs(waiting, "waiting"),
                cs(running, "running"),
                cs(complete, "complete"),
                cs(failed, "failed"),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;

    /// Helper to create test environment with tasks
    async fn setup_test_environment_with_tasks(task_count: usize) -> anyhow::Result<AppContext> {
        use crate::task::Task;

        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories()?;

        // Create test repository for task data (but not for the command execution context)
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();

        // Add tasks via storage API
        let storage = get_task_storage(tsk_env.clone());
        for i in 0..task_count {
            let task_id = format!("task-{}", i + 1);
            let task_dir_path = tsk_env.task_dir(&task_id);
            std::fs::create_dir_all(&task_dir_path)?;

            let instructions_path = task_dir_path.join("instructions.md");
            std::fs::write(&instructions_path, format!("Task {} instructions", i + 1))?;

            let status = match i % 4 {
                0 => TaskStatus::Queued,
                1 => TaskStatus::Running,
                2 => TaskStatus::Complete,
                _ => TaskStatus::Failed,
            };

            let started_at = if matches!(
                status,
                TaskStatus::Running | TaskStatus::Complete | TaskStatus::Failed
            ) {
                Some(chrono::Utc::now())
            } else {
                None
            };
            let completed_at = if matches!(status, TaskStatus::Complete | TaskStatus::Failed) {
                Some(chrono::Utc::now())
            } else {
                None
            };
            let task = Task {
                id: task_id.clone(),
                repo_root: repo_root.clone(),
                name: format!("task-{}", i + 1),
                instructions_file: instructions_path.to_string_lossy().to_string(),
                branch_name: format!("tsk/feat/task-{}/{}", i + 1, task_id),
                copied_repo_path: Some(task_dir_path),
                status,
                started_at,
                completed_at,
                ..Task::test_default()
            };
            storage
                .add_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        Ok(ctx)
    }

    #[tokio::test]
    async fn test_list_command_no_tasks() {
        let ctx = setup_test_environment_with_tasks(0).await.unwrap();

        let cmd = ListCommand;
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_command_with_tasks() {
        let ctx = setup_test_environment_with_tasks(4).await.unwrap();

        let cmd = ListCommand;
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_command_verifies_task_counts() {
        let ctx = setup_test_environment_with_tasks(8).await.unwrap();
        let tsk_env = ctx.tsk_env();

        // Verify the tasks were created correctly
        let storage = get_task_storage(tsk_env);
        let tasks = storage.list_tasks().await.unwrap();

        assert_eq!(tasks.len(), 8);

        // Check the distribution of task statuses (based on our setup logic)
        let queued_count = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .count();
        let running_count = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running)
            .count();
        let complete_count = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Complete)
            .count();
        let failed_count = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Failed)
            .count();

        assert_eq!(queued_count, 2); // tasks 0 and 4 (indices % 4 == 0)
        assert_eq!(running_count, 2); // tasks 1 and 5 (indices % 4 == 1)
        assert_eq!(complete_count, 2); // tasks 2 and 6 (indices % 4 == 2)
        assert_eq!(failed_count, 2); // tasks 3 and 7 (indices % 4 == 3)

        // Execute the list command
        let cmd = ListCommand;
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());
    }
}
