use is_terminal::IsTerminal;
use std::io::{self, Read};

/// Reads from stdin if data is piped, returns None if stdin is a TTY
///
/// This function detects whether stdin is connected to a terminal or has data piped to it.
/// If data is piped, it reads all the data and returns it as a trimmed string.
/// If stdin is a TTY (normal terminal), it returns None without blocking.
///
/// # Returns
///
/// - `Ok(Some(String))` - Data was piped and successfully read
/// - `Ok(None)` - No data piped (stdin is a TTY) or piped data was empty after trimming
/// - `Err(io::Error)` - Error reading from stdin
pub fn read_piped_input() -> Result<Option<String>, io::Error> {
    if !io::stdin().is_terminal() {
        // Show feedback when reading from stdin
        eprintln!("Reading task description from stdin...");

        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;

        let trimmed = buffer.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            // Show how much data was read
            eprintln!("Read {} characters from stdin", trimmed.len());
            Ok(Some(trimmed.to_string()))
        }
    } else {
        Ok(None)
    }
}

/// Merges piped input with existing description, warning if both exist
///
/// This function handles the priority logic for combining CLI --description flag
/// with piped stdin input. Piped input takes precedence over the CLI flag.
///
/// # Arguments
///
/// - `cli_description` - Description provided via --description flag
/// - `piped_input` - Description read from stdin pipe
pub fn merge_description_with_stdin(
    cli_description: Option<String>,
    piped_input: Option<String>,
) -> Option<String> {
    match (cli_description, piped_input) {
        (Some(_), Some(piped)) => {
            eprintln!(
                "Warning: Both --description flag and piped input provided. Using piped input."
            );
            Some(piped)
        }
        (None, Some(piped)) => Some(piped),
        (Some(desc), None) => Some(desc),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::test_utils::TestGitRepository;

    #[test]
    fn test_merge_description_with_stdin() {
        // Test case 1: Both CLI and piped - piped wins
        let result = merge_description_with_stdin(
            Some("cli desc".to_string()),
            Some("piped desc".to_string()),
        );
        assert_eq!(result, Some("piped desc".to_string()));

        // Test case 2: Only piped input
        let result = merge_description_with_stdin(None, Some("piped desc".to_string()));
        assert_eq!(result, Some("piped desc".to_string()));

        // Test case 3: Only CLI description
        let result = merge_description_with_stdin(Some("cli desc".to_string()), None);
        assert_eq!(result, Some("cli desc".to_string()));

        // Test case 4: No description
        let result = merge_description_with_stdin(None, None);
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_add_command_with_piped_input() {
        use crate::commands::AddCommand;
        use crate::commands::Command;
        use crate::commands::task_args::TaskArgs;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create a template file
        let template_content = "# Task: {{TYPE}}\n{{DESCRIPTION}}";
        test_repo
            .create_file(".tsk/templates/generic.md", template_content)
            .unwrap();

        // Create AppContext
        let ctx = AppContext::builder().build();

        // Test 1: Command without description should fail normally (when not piped)
        let cmd = AddCommand {
            task_args: TaskArgs {
                name: Some("test-no-desc".to_string()),
                r#type: "generic".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        // Should fail without piped input
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());

        // Test 2: Command with CLI description should work
        let cmd_with_desc = AddCommand {
            task_args: TaskArgs {
                name: Some("test-with-desc".to_string()),
                r#type: "generic".to_string(),
                description: Some("CLI description".to_string()),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            parent_id: None,
        };

        let result = cmd_with_desc.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Command with CLI description should succeed"
        );
    }

    #[tokio::test]
    async fn test_run_command_with_piped_input() {
        use crate::commands::Command;
        use crate::commands::RunCommand;
        use crate::commands::task_args::TaskArgs;
        use crate::test_utils::NoOpDockerClient;
        use std::sync::Arc;

        // Create a test git repository
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init_with_commit().unwrap();

        // Create a template file without {{DESCRIPTION}} placeholder
        let template_content = "Say ack and exit.";
        test_repo
            .create_file(".tsk/templates/ack.md", template_content)
            .unwrap();

        // Create AppContext
        let ctx = AppContext::builder().build();

        // Test: Command without description should work for templates without placeholder
        let cmd = RunCommand {
            task_args: TaskArgs {
                name: Some("test-ack".to_string()),
                r#type: "ack".to_string(),
                repo: Some(test_repo.path().to_string_lossy().to_string()),
                ..Default::default()
            },
            docker_client_override: Some(Arc::new(NoOpDockerClient)),
        };

        // Should succeed for templates without {{DESCRIPTION}} placeholder
        let result = cmd.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "Command should succeed for template without placeholder"
        );
    }

    #[tokio::test]
    async fn test_shell_command_structure() {
        use crate::commands::ShellCommand;
        use crate::commands::task_args::TaskArgs;

        let cmd = ShellCommand {
            task_args: TaskArgs {
                name: Some("test-shell".to_string()),
                r#type: "generic".to_string(),
                description: Some("Test description".to_string()),
                agent: Some("claude".to_string()),
                ..Default::default()
            },
        };

        let args = &cmd.task_args;
        assert_eq!(args.resolved_name(), "test-shell");
        assert_eq!(args.r#type, "generic");
        assert_eq!(args.description, Some("Test description".to_string()));
        assert_eq!(args.prompt, None);
        assert!(!args.edit);
        assert_eq!(args.agent, Some("claude".to_string()));
        assert_eq!(args.stack, None);
        assert_eq!(args.project, None);
        assert_eq!(args.repo, None);
    }
}
