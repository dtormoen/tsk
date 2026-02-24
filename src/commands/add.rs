use super::Command;
use super::task_args::TaskArgs;
use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;
use std::path::PathBuf;

pub struct AddCommand {
    pub task_args: TaskArgs,
    pub parent_id: Option<String>,
}

#[async_trait]
impl Command for AddCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let args = &self.task_args;
        let name = args.resolved_name();

        let agents = args.parse_and_validate_agents()?;
        let prompt = args.resolve_prompt()?;
        let repo_root = args.resolve_repo_root()?;

        // Create tasks for each agent
        let mut created_tasks = Vec::new();
        let mut first_task_instructions: Option<String> = None;

        for (idx, agent) in agents.iter().enumerate() {
            let task = if idx == 0 {
                // First task: create normally
                let task = args
                    .configure_builder(
                        repo_root.clone(),
                        name.clone(),
                        Some(agent.clone()),
                        prompt.clone(),
                    )
                    .parent_id(self.parent_id.clone())
                    .build(ctx)
                    .await?;

                // Capture instructions for reuse if in edit mode
                if args.edit {
                    first_task_instructions = Some(
                        crate::file_system::read_file(&PathBuf::from(&task.instructions_file))
                            .await?,
                    );
                }

                task
            } else {
                // Subsequent tasks: reuse instructions if edit mode
                let builder = args
                    .configure_builder(
                        repo_root.clone(),
                        name.clone(),
                        Some(agent.clone()),
                        if first_task_instructions.is_some() {
                            None
                        } else {
                            prompt.clone()
                        },
                    )
                    .parent_id(self.parent_id.clone());

                if let Some(ref instructions_content) = first_task_instructions {
                    // Write instructions to temporary file for this task
                    let tasks_file = ctx.tsk_env().tasks_file();
                    let data_dir = tasks_file
                        .parent()
                        .ok_or("Unable to determine data directory")?;
                    let temp_instructions =
                        data_dir.join(format!("temp_instructions_{}.md", nanoid::nanoid!(8)));
                    crate::file_system::write_file(&temp_instructions, instructions_content)
                        .await?;

                    // Disable edit mode — instructions are already finalized from the first task
                    let task = builder
                        .edit(false)
                        .existing_instructions_file(Some(temp_instructions.clone()))
                        .build(ctx)
                        .await?;

                    // Clean up temporary file after task is created
                    crate::file_system::remove_file(&temp_instructions)
                        .await
                        .ok();

                    task
                } else {
                    // Non-edit mode or no instructions captured
                    builder.build(ctx).await?
                }
            };

            let storage = ctx.task_storage();
            storage
                .add_task(task.clone())
                .await
                .map_err(|e| e as Box<dyn Error>)?;

            created_tasks.push(task);
        }

        // Print compact summary
        let parent_suffix = |task: &crate::task::Task| -> String {
            if task.parent_ids.is_empty() {
                String::new()
            } else {
                format!(" parent:{}", task.parent_ids.join(","))
            }
        };

        if created_tasks.len() == 1 {
            let task = &created_tasks[0];
            println!(
                "Queued {} ({}, {}, {}){}",
                task.id,
                task.task_type,
                task.stack,
                task.agent,
                parent_suffix(task)
            );
        } else {
            let first = &created_tasks[0];
            println!(
                "Queued {} tasks ({}, {}):",
                created_tasks.len(),
                first.task_type,
                first.stack
            );
            for task in &created_tasks {
                println!("  {} ({}){}", task.id, task.agent, parent_suffix(task));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> AppContext {
        AppContext::builder().build()
    }

    #[tokio::test]
    async fn test_add_command_validation_no_input() {
        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test".to_string()),
                r#type: "generic".to_string(),
                repo: Some(".".to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Either prompt or prompt file must be provided, or use edit mode"),
            "Expected validation error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_add_command_invalid_task_type() {
        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test".to_string()),
                r#type: "nonexistent".to_string(),
                prompt: Some("test description".to_string()),
                repo: Some(".".to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("No template found for task type 'nonexistent'"),
            "Expected template error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_add_command_template_without_prompt() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-ack".to_string()),
                r#type: "ack".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Command should succeed for template without prompt placeholder"
        );
    }

    #[tokio::test]
    async fn test_add_command_with_repo_path() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "# Task: {{TYPE}}\n{{PROMPT}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-repo-path".to_string()),
                r#type: "generic".to_string(),
                prompt: Some("Test with repo path".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let current_dir = std::env::current_dir().unwrap();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Command should succeed with repo path");

        assert_eq!(
            std::env::current_dir().unwrap(),
            current_dir,
            "Current directory should not have changed"
        );
    }

    #[tokio::test]
    async fn test_add_command_multiple_agents() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "# Task: {{TYPE}}\n{{PROMPT}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-multi".to_string()),
                r#type: "generic".to_string(),
                prompt: Some("Test with multiple agents".to_string()),
                agent: Some("codex,claude".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Should create tasks for multiple agents");

        let storage = ctx.task_storage();
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        let agent_names: Vec<&str> = tasks.iter().map(|t| t.agent.as_str()).collect();
        assert!(agent_names.contains(&"codex"));
        assert!(agent_names.contains(&"claude"));

        assert_eq!(tasks[0].name, "test-multi");
        assert_eq!(tasks[1].name, "test-multi");
    }

    #[tokio::test]
    async fn test_add_command_invalid_agent_in_list() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-invalid".to_string()),
                r#type: "generic".to_string(),
                prompt: Some("Test description".to_string()),
                agent: Some("claude,invalid-agent,codex".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_err(), "Should fail for invalid agent");

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Unknown agent"));
        assert!(err_msg.contains("invalid-agent"));
    }

    #[tokio::test]
    async fn test_add_command_duplicate_agents() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-duplicate".to_string()),
                r#type: "generic".to_string(),
                prompt: Some("Test with duplicate agents".to_string()),
                agent: Some("codex,codex".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Should create tasks for duplicate agents");

        let storage = ctx.task_storage();
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        assert_eq!(tasks[0].agent, "codex");
        assert_eq!(tasks[1].agent, "codex");
    }

    #[tokio::test]
    async fn test_add_command_name_defaults_to_type() {
        use crate::test_utils::TestGitRepository;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "# Task: {{TYPE}}\n{{PROMPT}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                r#type: "generic".to_string(),
                prompt: Some("Test description".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Command should succeed: {:?}", result.err());

        let storage = ctx.task_storage();
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1, "Should create 1 task");
        assert_eq!(
            tasks[0].name, "generic",
            "Task name should default to type value"
        );
    }

    #[tokio::test]
    async fn test_add_command_multiple_agents_with_edit() {
        use crate::context::tsk_env::TskEnv;
        use crate::test_utils::TestGitRepository;
        use std::sync::Arc;

        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        let template_content = "# Task: {{TYPE}}\n{{PROMPT}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let script_path = test_repo.path().join("mock_editor.sh");
        std::fs::write(
            &script_path,
            "#!/bin/sh\nprintf 'Test instructions from editor\\n' >> \"$1\"\n",
        )
        .unwrap();
        std::fs::set_permissions(
            &script_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let temp_path = temp_dir.path();

        let tsk_env = Arc::new(
            TskEnv::builder()
                .with_data_dir(temp_path.join("data"))
                .with_runtime_dir(temp_path.join("runtime"))
                .with_config_dir(temp_path.join("config"))
                .with_claude_config_dir(temp_path.join("claude"))
                .with_editor(script_path.to_str().unwrap().to_string())
                .build()
                .unwrap(),
        );
        tsk_env.ensure_directories().unwrap();

        let ctx = AppContext::builder().with_tsk_env(tsk_env).build();

        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-multi-edit".to_string()),
                r#type: "generic".to_string(),
                prompt: Some("Test with multiple agents in edit mode".to_string()),
                edit: true,
                agent: Some("codex,claude".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;

        assert!(
            result.is_ok(),
            "Should create tasks with edit mode: {:?}",
            result.err()
        );

        let storage = ctx.task_storage();
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        let agent_names: Vec<&str> = tasks.iter().map(|t| t.agent.as_str()).collect();
        assert!(agent_names.contains(&"codex"));
        assert!(agent_names.contains(&"claude"));

        let instructions_1 =
            crate::file_system::read_file(&PathBuf::from(&tasks[0].instructions_file))
                .await
                .unwrap();
        let instructions_2 =
            crate::file_system::read_file(&PathBuf::from(&tasks[1].instructions_file))
                .await
                .unwrap();

        assert_eq!(
            instructions_1, instructions_2,
            "Both tasks should have identical instructions"
        );
        assert!(
            instructions_1.contains("Test instructions from editor"),
            "Instructions should contain editor-written content"
        );
    }
}
