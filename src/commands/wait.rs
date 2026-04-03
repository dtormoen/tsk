use super::Command;
use crate::agent::log_line::LogLine;
use crate::context::AppContext;
use crate::display::{colorize_status, format_duration};
use crate::task::TaskStatus;
use async_trait::async_trait;
use is_terminal::IsTerminal;
use std::error::Error;
use std::io::BufRead;

pub struct WaitCommand {
    pub task_ids: Vec<String>,
}

#[async_trait]
impl Command for WaitCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        wait_for_tasks(ctx, &self.task_ids).await
    }
}

/// Blocks until all tasks reach a terminal state, printing summaries as each completes.
///
/// Returns `Ok(())` if all tasks completed successfully, `Err` if any failed or were cancelled.
pub async fn wait_for_tasks(ctx: &AppContext, task_ids: &[String]) -> Result<(), Box<dyn Error>> {
    if task_ids.is_empty() {
        return Err("No task IDs provided".into());
    }

    let mut pending: Vec<String> = task_ids.to_vec();
    let mut any_failed = false;

    while !pending.is_empty() {
        let mut still_pending = Vec::new();

        for task_id in &pending {
            let storage = ctx.task_storage();
            let task = storage
                .get_task(task_id)
                .await
                .map_err(|e| e as Box<dyn Error>)?;

            let task = match task {
                Some(t) => t,
                None => return Err(format!("Task {task_id} not found").into()),
            };

            match &task.status {
                TaskStatus::Complete => {
                    print_task_summary(ctx, &task);
                }
                TaskStatus::Failed | TaskStatus::Cancelled => {
                    print_task_summary(ctx, &task);
                    any_failed = true;
                }
                TaskStatus::Queued | TaskStatus::Running => {
                    if task.is_blocked() {
                        eprintln!(
                            "Task {} ({}) is blocked: {}",
                            task.name,
                            task.id,
                            task.blocked_reason.as_ref().unwrap()
                        );
                        any_failed = true;
                    } else {
                        still_pending.push(task_id.clone());
                    }
                }
            }
        }

        pending = still_pending;

        if !pending.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    if any_failed {
        Err("One or more tasks failed or were cancelled".into())
    } else {
        Ok(())
    }
}

/// Prints a summary of a completed task including status, branch, duration, and recent output.
fn print_task_summary(ctx: &AppContext, task: &crate::task::Task) {
    let styled = std::io::stdout().is_terminal();

    let status_str = match &task.status {
        TaskStatus::Complete => "COMPLETE",
        TaskStatus::Failed => "FAILED",
        TaskStatus::Cancelled => "CANCELLED",
        TaskStatus::Queued => "QUEUED",
        TaskStatus::Running => "RUNNING",
    };

    println!(
        "Task {} ({}) {}",
        task.name,
        task.id,
        colorize_status(status_str, styled)
    );
    println!("  Branch: {}", task.branch_name);

    let duration = match (&task.started_at, &task.completed_at) {
        (Some(start), Some(end)) => format_duration((*end - *start).num_seconds()),
        _ => "-".to_string(),
    };
    println!("  Duration: {duration}");

    if let TaskStatus::Failed = &task.status
        && let Some(ref err) = task.error_message
    {
        println!("  Error: {err}");
    }

    let log_path = ctx.tsk_env().task_dir(&task.id).join("output/agent.log");

    if let Ok(file) = std::fs::File::open(&log_path) {
        let lines: Vec<String> = std::io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .collect();

        let last_lines = if lines.len() > 10 {
            &lines[lines.len() - 10..]
        } else {
            &lines
        };

        let mut summary_line: Option<LogLine> = None;
        let mut display_lines: Vec<String> = Vec::new();

        for line in last_lines {
            if let Ok(log_line) = serde_json::from_str::<LogLine>(line) {
                if matches!(&log_line, LogLine::Summary { .. }) {
                    summary_line = Some(log_line);
                } else {
                    display_lines.push(log_line.to_string());
                }
            }
        }

        if !display_lines.is_empty() {
            for line in &display_lines {
                println!("  {line}");
            }
        }

        if let Some(summary) = summary_line {
            println!("  {summary}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Task, TaskStatus};

    fn create_test_context() -> AppContext {
        AppContext::builder().build()
    }

    #[tokio::test]
    async fn test_wait_already_complete() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        let task = Task {
            id: "wait-complete-1".to_string(),
            status: TaskStatus::Complete,
            started_at: Some(chrono::Utc::now()),
            completed_at: Some(chrono::Utc::now()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let result = wait_for_tasks(&ctx, &["wait-complete-1".to_string()]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_already_failed() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        let task = Task {
            id: "wait-failed-1".to_string(),
            status: TaskStatus::Failed,
            error_message: Some("something went wrong".to_string()),
            started_at: Some(chrono::Utc::now()),
            completed_at: Some(chrono::Utc::now()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let result = wait_for_tasks(&ctx, &["wait-failed-1".to_string()]).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("failed"),
            "Expected 'failed' in: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_wait_task_not_found() {
        let ctx = create_test_context();

        let result = wait_for_tasks(&ctx, &["nonexistent-task-id".to_string()]).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found"),
            "Expected 'not found' in: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_wait_already_cancelled() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        let task = Task {
            id: "wait-cancelled-1".to_string(),
            status: TaskStatus::Cancelled,
            started_at: Some(chrono::Utc::now()),
            completed_at: Some(chrono::Utc::now()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let result = wait_for_tasks(&ctx, &["wait-cancelled-1".to_string()]).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cancelled"),
            "Expected 'cancelled' in: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_wait_for_tasks_all_complete() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        for i in 1..=3 {
            let task = Task {
                id: format!("wait-multi-ok-{i}"),
                status: TaskStatus::Complete,
                started_at: Some(chrono::Utc::now()),
                completed_at: Some(chrono::Utc::now()),
                ..Task::test_default()
            };
            storage.add_task(task).await.unwrap();
        }

        let ids: Vec<String> = (1..=3).map(|i| format!("wait-multi-ok-{i}")).collect();
        let result = wait_for_tasks(&ctx, &ids).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_blocked_task_exits_with_error() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        let task = Task {
            id: "wait-blocked-1".to_string(),
            blocked_reason: Some("Agent warmup failed: OAuth expired".to_string()),
            ..Task::test_default()
        };
        storage.add_task(task).await.unwrap();

        let result = wait_for_tasks(&ctx, &["wait-blocked-1".to_string()]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_wait_for_tasks_some_failed() {
        let ctx = create_test_context();
        let storage = ctx.task_storage();

        let task1 = Task {
            id: "wait-mix-1".to_string(),
            status: TaskStatus::Complete,
            started_at: Some(chrono::Utc::now()),
            completed_at: Some(chrono::Utc::now()),
            ..Task::test_default()
        };
        storage.add_task(task1).await.unwrap();

        let task2 = Task {
            id: "wait-mix-2".to_string(),
            status: TaskStatus::Failed,
            started_at: Some(chrono::Utc::now()),
            completed_at: Some(chrono::Utc::now()),
            ..Task::test_default()
        };
        storage.add_task(task2).await.unwrap();

        let ids = vec!["wait-mix-1".to_string(), "wait-mix-2".to_string()];
        let result = wait_for_tasks(&ctx, &ids).await;
        assert!(result.is_err());
    }
}
