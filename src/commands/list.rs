use super::Command;
use crate::context::AppContext;
use crate::task::TaskStatus;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use tabled::settings::Style;
use tabled::{Table, Tabled};

pub struct ListCommand;

#[derive(Tabled)]
struct TaskRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Type")]
    task_type: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Agent")]
    agent: String,
    #[tabled(rename = "Branch")]
    branch: String,
    #[tabled(rename = "Created")]
    created: String,
}

#[async_trait]
impl Command for ListCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Try to get tasks from server first
        let client = ctx.tsk_client();
        let tasks = if client.is_server_available().await {
            match client.list_tasks().await {
                Ok(tasks) => tasks,
                Err(_) => {
                    eprintln!("Failed to list tasks via server");
                    eprintln!("Falling back to direct file read...");

                    // Fall back to direct storage
                    let storage = get_task_storage(ctx.tsk_config(), ctx.file_system());
                    storage
                        .list_tasks()
                        .await
                        .map_err(|e| e as Box<dyn Error>)?
                }
            }
        } else {
            // Server not available, read directly
            let storage = get_task_storage(ctx.tsk_config(), ctx.file_system());
            storage
                .list_tasks()
                .await
                .map_err(|e| e as Box<dyn Error>)?
        };

        if tasks.is_empty() {
            println!("No tasks in queue");
        } else {
            let rows: Vec<TaskRow> = tasks
                .iter()
                .map(|task| TaskRow {
                    id: task.id.clone(),
                    name: task.name.clone(),
                    task_type: task.task_type.clone(),
                    status: match &task.status {
                        TaskStatus::Queued => "QUEUED".to_string(),
                        TaskStatus::Running => "RUNNING".to_string(),
                        TaskStatus::Failed => "FAILED".to_string(),
                        TaskStatus::Complete => "COMPLETE".to_string(),
                    },
                    agent: task.agent.clone(),
                    branch: task.branch_name.clone(),
                    created: task.created_at.format("%Y-%m-%d %H:%M").to_string(),
                })
                .collect();

            let table = Table::new(rows).with(Style::modern()).to_string();
            println!("{table}");

            // Print summary
            let queued = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Queued)
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

            println!(
                "\nSummary: {queued} queued, {running} running, {complete} complete, {failed} failed"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::DefaultFileSystem;
    use crate::task_storage::get_task_storage;
    use crate::test_utils::TestGitRepository;
    use std::sync::Arc;

    /// Helper to create test environment with tasks
    async fn setup_test_environment_with_tasks(task_count: usize) -> anyhow::Result<AppContext> {
        // Create AppContext with test defaults
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        config.ensure_directories()?;

        // Create test repository for task data (but not for the command execution context)
        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();
        let repo_hash = crate::storage::get_repo_hash(&repo_root);

        // Create sample tasks
        let mut tasks_json = Vec::new();
        for i in 0..task_count {
            let task_id = format!("task-{}", i + 1);
            let task_dir_path = config.task_dir(&task_id, &repo_hash);
            std::fs::create_dir_all(&task_dir_path)?;

            let instructions_path = task_dir_path.join("instructions.md");
            std::fs::write(&instructions_path, format!("Task {} instructions", i + 1))?;

            let status = match i % 4 {
                0 => "QUEUED",
                1 => "RUNNING",
                2 => "COMPLETE",
                _ => "FAILED",
            };

            tasks_json.push(format!(
                r#"{{"id":"{}","repo_root":"{}","name":"task-{}","task_type":"feat","instructions_file":"{}","agent":"claude","timeout":30,"status":"{}","created_at":"2024-01-01T12:{:02}:00Z","started_at":null,"completed_at":null,"branch_name":"tsk/feat/task-{}/{}","error_message":null,"source_commit":"abc123","stack":"default","project":"default","copied_repo_path":"{}"}}"#,
                task_id,
                repo_root.to_string_lossy(),
                i + 1,
                instructions_path.to_string_lossy(),
                status,
                i,
                i + 1,
                task_id,
                task_dir_path.to_string_lossy()
            ));
        }

        // Write tasks.json
        let tasks_json_content = format!("[{}]", tasks_json.join(","));
        let tasks_file_path = config.tasks_file();
        if let Some(parent) = tasks_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&tasks_file_path, &tasks_json_content)?;

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
        let config = ctx.tsk_config();

        // Verify the tasks were created correctly
        let file_system = Arc::new(DefaultFileSystem);
        let storage = get_task_storage(config, file_system);
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
