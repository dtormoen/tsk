use super::Command;
use crate::context::AppContext;
use crate::task::{get_task_storage, TaskStatus};
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
    #[tabled(rename = "Created")]
    created: String,
}

#[async_trait]
impl Command for ListCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let storage = get_task_storage(ctx.file_system());
        let tasks = storage.list_tasks().await?;

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
                    agent: task.agent.clone().unwrap_or_else(|| "auto".to_string()),
                    created: task.created_at.format("%Y-%m-%d %H:%M").to_string(),
                })
                .collect();

            let table = Table::new(rows).with(Style::modern()).to_string();
            println!("{}", table);

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
                "\nSummary: {} queued, {} running, {} complete, {} failed",
                queued, running, complete, failed
            );
        }

        Ok(())
    }
}
