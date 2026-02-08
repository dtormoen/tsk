use super::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use crate::stdin_utils::{merge_description_with_stdin, read_piped_input};
use crate::task::TaskBuilder;
use crate::task_storage::get_task_storage;
use async_trait::async_trait;
use std::error::Error;
use std::path::{Path, PathBuf};

pub struct AddCommand {
    pub name: Option<String>,
    pub r#type: String,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub edit: bool,
    pub agent: Option<String>,
    pub stack: Option<String>,
    pub project: Option<String>,
    pub repo: Option<String>,
    pub parent_id: Option<String>,
}

#[async_trait]
impl Command for AddCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Resolve name: use provided name or default to task type
        let name = self.name.clone().unwrap_or_else(|| self.r#type.clone());

        println!("Adding task to queue: {}", name);

        // Parse comma-separated agents or use default
        let agents: Vec<String> = match &self.agent {
            Some(agent_str) => agent_str.split(',').map(|s| s.trim().to_string()).collect(),
            None => vec![crate::agent::AgentProvider::default_agent().to_string()],
        };

        // Validate all agents before creating any tasks
        for agent in &agents {
            if !crate::agent::AgentProvider::is_valid_agent(agent) {
                let available_agents = crate::agent::AgentProvider::list_agents().join(", ");
                return Err(format!(
                    "Unknown agent '{}'. Available agents: {}",
                    agent, available_agents
                )
                .into());
            }
        }

        // Read from stdin if data is piped
        let piped_input = read_piped_input()?;

        // Merge piped input with CLI description (piped input takes precedence)
        let final_description = merge_description_with_stdin(self.description.clone(), piped_input);

        // Find repository root
        let start_path = self.repo.as_deref().unwrap_or(".");
        let repo_root = find_repository_root(Path::new(start_path))?;

        // Create tasks for each agent
        let mut created_tasks = Vec::new();
        let mut first_task_instructions: Option<String> = None;

        for (idx, agent) in agents.iter().enumerate() {
            let task = if idx == 0 {
                // First task: create normally
                let task = TaskBuilder::new()
                    .repo_root(repo_root.clone())
                    .name(name.clone())
                    .task_type(self.r#type.clone())
                    .description(final_description.clone())
                    .instructions_file(self.prompt.as_ref().map(PathBuf::from))
                    .edit(self.edit)
                    .agent(Some(agent.clone()))
                    .stack(self.stack.clone())
                    .project(self.project.clone())
                    .parent_id(self.parent_id.clone())
                    .build(ctx)
                    .await?;

                // Capture instructions for reuse if in edit mode
                if self.edit {
                    first_task_instructions = Some(
                        ctx.file_system()
                            .read_file(&PathBuf::from(&task.instructions_file))
                            .await?,
                    );
                }

                task
            } else {
                // Subsequent tasks: reuse instructions if edit mode
                let builder = TaskBuilder::new()
                    .repo_root(repo_root.clone())
                    .name(name.clone())
                    .task_type(self.r#type.clone())
                    .agent(Some(agent.clone()))
                    .stack(self.stack.clone())
                    .project(self.project.clone())
                    .parent_id(self.parent_id.clone());

                if let Some(ref instructions_content) = first_task_instructions {
                    // Write instructions to temporary file for this task
                    // Use parent of tasks_file to get data_dir (since data_dir() is test-only)
                    let tasks_file = ctx.tsk_env().tasks_file();
                    let data_dir = tasks_file
                        .parent()
                        .ok_or("Unable to determine data directory")?;
                    let temp_instructions =
                        data_dir.join(format!("temp_instructions_{}.md", nanoid::nanoid!(8)));
                    ctx.file_system()
                        .write_file(&temp_instructions, instructions_content)
                        .await?;

                    let task = builder
                        .instructions_file(Some(temp_instructions.clone()))
                        .build(ctx)
                        .await?;

                    // Clean up temporary file after task is created
                    ctx.file_system().remove_file(&temp_instructions).await.ok();

                    task
                } else {
                    // Non-edit mode or no instructions captured
                    builder
                        .description(final_description.clone())
                        .instructions_file(self.prompt.as_ref().map(PathBuf::from))
                        .build(ctx)
                        .await?
                }
            };

            let storage = get_task_storage(ctx.tsk_env());
            storage
                .add_task(task.clone())
                .await
                .map_err(|e| e as Box<dyn Error>)?;

            created_tasks.push(task);
        }

        // Print success messages for all created tasks
        if created_tasks.len() == 1 {
            // Single task - use original output format
            let task = &created_tasks[0];
            println!("\nTask successfully added to queue!");
            println!("Task ID: {}", task.id);
            println!("Type: {}", self.r#type);
            if let Some(ref desc) = final_description {
                println!("Description: {desc}");
            }
            if self.prompt.is_some() {
                println!("Prompt: Copied to task directory");
            }
            println!("Agent: {}", task.agent);
        } else {
            // Multiple tasks - show summary
            println!(
                "\n{} tasks successfully added to queue!",
                created_tasks.len()
            );
            println!("Type: {}", self.r#type);
            if let Some(ref desc) = final_description {
                println!("Description: {desc}");
            }
            println!("\nCreated tasks:");
            for task in &created_tasks {
                println!("  - Task ID: {} (Agent: {})", task.id, task.agent);
            }
        }

        println!("\nUse 'tsk list' to view all queued tasks");
        println!("Use 'tsk run' to execute the next task in the queue");

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
            name: Some("test".to_string()),
            r#type: "generic".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(".".to_string()),
            parent_id: None,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg
                .contains("Either description or prompt file must be provided, or use edit mode"),
            "Expected validation error, but got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_add_command_invalid_task_type() {
        let cmd = AddCommand {
            name: Some("test".to_string()),
            r#type: "nonexistent".to_string(),
            description: Some("test description".to_string()),
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(".".to_string()),
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
    async fn test_add_command_template_without_description() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create template file without {{DESCRIPTION}} placeholder
        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        // Create AppContext - automatically gets test defaults
        let ctx = AppContext::builder().build();

        // Create AddCommand without description (should succeed for templates without placeholder)
        let cmd = AddCommand {
            name: Some("test-ack".to_string()),
            r#type: "ack".to_string(),
            description: None,
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        // Execute should succeed
        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Command should succeed for template without description placeholder"
        );
    }

    #[tokio::test]
    async fn test_add_command_with_repo_path() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create a template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        // Create AppContext - automatically gets test defaults
        let ctx = AppContext::builder().build();

        // Create AddCommand with repo path
        let cmd = AddCommand {
            name: Some("test-repo-path".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test with repo path".to_string()),
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        // Execute should succeed without changing directories
        let current_dir = std::env::current_dir().unwrap();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Command should succeed with repo path");

        // Verify we didn't change directories
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

        // Create template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        let cmd = AddCommand {
            name: Some("test-multi".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test with multiple agents".to_string()),
            prompt: None,
            edit: false,
            agent: Some("codex,claude".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Should create tasks for multiple agents");

        // Verify tasks were created
        let storage = crate::task_storage::get_task_storage(ctx.tsk_env());
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        // Verify agent names
        let agent_names: Vec<&str> = tasks.iter().map(|t| t.agent.as_str()).collect();
        assert!(agent_names.contains(&"codex"));
        assert!(agent_names.contains(&"claude"));

        // Verify same name for all tasks
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
            name: Some("test-invalid".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: Some("claude,invalid-agent,codex".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
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
            name: Some("test-duplicate".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test with duplicate agents".to_string()),
            prompt: None,
            edit: false,
            agent: Some("codex,codex".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Should create tasks for duplicate agents");

        // Verify tasks were created
        let storage = crate::task_storage::get_task_storage(ctx.tsk_env());
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        // Both should use codex
        assert_eq!(tasks[0].agent, "codex");
        assert_eq!(tasks[1].agent, "codex");
    }

    #[tokio::test]
    async fn test_add_command_name_defaults_to_type() {
        use crate::test_utils::TestGitRepository;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        let ctx = AppContext::builder().build();

        // Create AddCommand with name: None - should default to type value
        let cmd = AddCommand {
            name: None,
            r#type: "generic".to_string(),
            description: Some("Test description".to_string()),
            prompt: None,
            edit: false,
            agent: None,
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;
        assert!(result.is_ok(), "Command should succeed: {:?}", result.err());

        // Verify the created task has name defaulted to type
        let storage = crate::task_storage::get_task_storage(ctx.tsk_env());
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

        // Create template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        // Set EDITOR to a script that writes instructions and exits
        // Create a simple shell script that appends content
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

        // Create a temporary directory for test configuration
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_path = temp_dir.path();

        // Create a custom TskEnv with the mock editor
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
            name: Some("test-multi-edit".to_string()),
            r#type: "generic".to_string(),
            description: Some("Test with multiple agents in edit mode".to_string()),
            prompt: None,
            edit: true,
            agent: Some("codex,claude".to_string()),
            stack: None,
            project: None,
            repo: Some(test_repo.path().to_string_lossy().to_string()),
            parent_id: None,
        };

        let result = cmd.execute(&ctx).await;

        assert!(
            result.is_ok(),
            "Should create tasks with edit mode: {:?}",
            result.err()
        );

        // Verify tasks were created
        let storage = crate::task_storage::get_task_storage(ctx.tsk_env());
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 2, "Should create 2 tasks");

        // Verify agent names
        let agent_names: Vec<&str> = tasks.iter().map(|t| t.agent.as_str()).collect();
        assert!(agent_names.contains(&"codex"));
        assert!(agent_names.contains(&"claude"));

        // Verify both tasks have the same instructions content
        let instructions_1 = ctx
            .file_system()
            .read_file(&PathBuf::from(&tasks[0].instructions_file))
            .await
            .unwrap();
        let instructions_2 = ctx
            .file_system()
            .read_file(&PathBuf::from(&tasks[1].instructions_file))
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
