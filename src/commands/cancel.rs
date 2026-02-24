use super::Command;
use crate::context::AppContext;
use crate::context::docker_client::{DefaultDockerClient, DockerClient};
use crate::task::TaskStatus;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

pub struct CancelCommand {
    pub task_ids: Vec<String>,
    /// Optional Docker client override for dependency injection
    pub docker_client_override: Option<Arc<dyn DockerClient>>,
}

#[async_trait]
impl Command for CancelCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.task_ids.is_empty() {
            return Err("No task IDs provided".into());
        }

        let storage = ctx.task_storage();
        let docker_client: Arc<dyn DockerClient> = match &self.docker_client_override {
            Some(client) => Arc::clone(client),
            None => Arc::new(
                DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                    .map_err(|e| -> Box<dyn Error> { e.into() })?,
            ),
        };

        let mut successful_cancels = 0;
        let mut failed_cancels = 0;

        for task_id in &self.task_ids {
            let task = match storage.get_task(task_id).await {
                Ok(Some(task)) => task,
                Ok(None) => {
                    eprintln!("Task {task_id} not found");
                    failed_cancels += 1;
                    continue;
                }
                Err(e) => {
                    eprintln!("Failed to look up {task_id}: {e}");
                    failed_cancels += 1;
                    continue;
                }
            };

            match task.status {
                TaskStatus::Queued | TaskStatus::Running => {
                    let was_running = task.status == TaskStatus::Running;
                    let is_interactive = task.is_interactive;

                    match storage.mark_cancelled(task_id).await {
                        Ok(_) => {
                            println!("Cancelled {task_id}");
                            successful_cancels += 1;

                            if was_running {
                                let container_name = if is_interactive {
                                    format!("tsk-interactive-{task_id}")
                                } else {
                                    format!("tsk-{task_id}")
                                };
                                if let Err(e) = docker_client.kill_container(&container_name).await
                                {
                                    eprintln!(
                                        "Note: Could not kill container {container_name}: {e}"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to cancel {task_id}: {e}");
                            failed_cancels += 1;
                        }
                    }
                }
                TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Cancelled => {
                    let status_str = match task.status {
                        TaskStatus::Complete => "COMPLETE",
                        TaskStatus::Failed => "FAILED",
                        TaskStatus::Cancelled => "CANCELLED",
                        _ => unreachable!(),
                    };
                    println!("Skipped {task_id}: already {status_str}");
                }
            }
        }

        if failed_cancels > 0 {
            if successful_cancels > 0 {
                println!("\n{successful_cancels} cancelled, {failed_cancels} failed");
            }
            return Err(format!("{failed_cancels} task(s) failed to cancel").into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{NoOpDockerClient, TestGitRepository};

    fn mock_docker_client() -> Option<Arc<dyn DockerClient>> {
        Some(Arc::new(NoOpDockerClient))
    }

    async fn setup_test_with_tasks(
        tasks: Vec<(&str, TaskStatus)>,
    ) -> anyhow::Result<(AppContext, TestGitRepository)> {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        tsk_env.ensure_directories()?;

        let test_repo = TestGitRepository::new()?;
        test_repo.init_with_commit()?;
        let repo_root = test_repo.path().to_path_buf();

        let storage = ctx.task_storage();
        for (i, (task_id, status)) in tasks.iter().enumerate() {
            let task_dir_path = tsk_env.task_dir(task_id);
            std::fs::create_dir_all(&task_dir_path)?;

            let mut task = crate::task::Task {
                id: task_id.to_string(),
                repo_root: repo_root.clone(),
                name: format!("test-task-{i}"),
                instructions_file: task_dir_path
                    .join("instructions.md")
                    .to_string_lossy()
                    .to_string(),
                branch_name: format!("tsk/{task_id}"),
                copied_repo_path: Some(task_dir_path),
                ..crate::task::Task::test_default()
            };
            task.status = TaskStatus::Queued;
            storage
                .add_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            match status {
                TaskStatus::Running => {
                    storage
                        .mark_running(task_id)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                TaskStatus::Complete => {
                    storage
                        .mark_running(task_id)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    storage
                        .mark_complete(task_id, &format!("tsk/{task_id}"))
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                TaskStatus::Failed => {
                    storage
                        .mark_running(task_id)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    storage
                        .mark_failed(task_id, "test failure")
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                TaskStatus::Cancelled => {
                    storage
                        .mark_cancelled(task_id)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                TaskStatus::Queued => {}
            }
        }

        Ok((ctx, test_repo))
    }

    #[tokio::test]
    async fn test_cancel_queued_task() {
        let (ctx, _repo) = setup_test_with_tasks(vec![("task-1", TaskStatus::Queued)])
            .await
            .unwrap();

        let cmd = CancelCommand {
            task_ids: vec!["task-1".to_string()],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        let storage = ctx.task_storage();
        let task = storage.get_task("task-1").await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_cancel_running_task() {
        let (ctx, _repo) = setup_test_with_tasks(vec![("task-1", TaskStatus::Running)])
            .await
            .unwrap();

        let cmd = CancelCommand {
            task_ids: vec!["task-1".to_string()],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        let storage = ctx.task_storage();
        let task = storage.get_task("task-1").await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_cancel_terminal_tasks_skips() {
        let (ctx, _repo) = setup_test_with_tasks(vec![
            ("task-complete", TaskStatus::Complete),
            ("task-failed", TaskStatus::Failed),
            ("task-cancelled", TaskStatus::Cancelled),
        ])
        .await
        .unwrap();

        let cmd = CancelCommand {
            task_ids: vec![
                "task-complete".to_string(),
                "task-failed".to_string(),
                "task-cancelled".to_string(),
            ],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "terminal tasks should be skipped, not errors"
        );

        let storage = ctx.task_storage();
        assert_eq!(
            storage
                .get_task("task-complete")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Complete
        );
        assert_eq!(
            storage
                .get_task("task-failed")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Failed
        );
        assert_eq!(
            storage
                .get_task("task-cancelled")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn test_cancel_multiple_mixed_tasks() {
        let (ctx, _repo) = setup_test_with_tasks(vec![
            ("task-queued", TaskStatus::Queued),
            ("task-complete", TaskStatus::Complete),
            ("task-running", TaskStatus::Running),
        ])
        .await
        .unwrap();

        let cmd = CancelCommand {
            task_ids: vec![
                "task-queued".to_string(),
                "task-complete".to_string(),
                "task-running".to_string(),
            ],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok());

        let storage = ctx.task_storage();
        assert_eq!(
            storage
                .get_task("task-queued")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Cancelled
        );
        assert_eq!(
            storage
                .get_task("task-complete")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Complete
        );
        assert_eq!(
            storage
                .get_task("task-running")
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn test_cancel_empty_task_ids() {
        let ctx = AppContext::builder().build();
        ctx.tsk_env().ensure_directories().unwrap();

        let cmd = CancelCommand {
            task_ids: vec![],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No task IDs provided");
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_task() {
        let ctx = AppContext::builder().build();
        ctx.tsk_env().ensure_directories().unwrap();

        let cmd = CancelCommand {
            task_ids: vec!["nonexistent".to_string()],
            docker_client_override: mock_docker_client(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 task(s) failed to cancel")
        );
    }
}
