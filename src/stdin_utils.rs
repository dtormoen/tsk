use std::io::{self, Read};
use std::os::unix::io::AsRawFd;

/// Returns true if stdin is a pipe (FIFO) or regular file redirect.
///
/// This is more precise than `!is_terminal()` because it correctly returns false
/// for unix sockets (e.g. when run from Claude Code's Bash tool), which would
/// block forever on read since they never send EOF.
fn stdin_is_pipe_or_file() -> bool {
    let fd = io::stdin().as_raw_fd();
    let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat_buf) } != 0 {
        return false;
    }
    let mode = stat_buf.st_mode & libc::S_IFMT;
    mode == libc::S_IFIFO || mode == libc::S_IFREG
}

/// Reads from stdin if data is piped, returns None otherwise
///
/// This function checks whether stdin is a pipe or file redirect. If so, it reads
/// all the data and returns it as a trimmed string. For terminals, sockets, and
/// other file descriptor types, it returns None without blocking.
///
/// # Returns
///
/// - `Ok(Some(String))` - Data was piped and successfully read
/// - `Ok(None)` - Stdin is not a pipe/file, or piped data was empty after trimming
/// - `Err(io::Error)` - Error reading from stdin
pub fn read_piped_input() -> Result<Option<String>, io::Error> {
    if stdin_is_pipe_or_file() {
        // Show feedback when reading from stdin
        eprintln!("Reading task prompt from stdin...");

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

/// Merges piped input with existing prompt, warning if both exist
///
/// This function handles the priority logic for combining CLI --description/--prompt
/// flag with piped stdin input. Piped input takes precedence over the CLI flag.
///
/// # Arguments
///
/// - `cli_prompt` - Prompt provided via --description or --prompt flag
/// - `piped_input` - Prompt read from stdin pipe
pub fn merge_prompt_with_stdin(
    cli_prompt: Option<String>,
    piped_input: Option<String>,
) -> Option<String> {
    match (cli_prompt, piped_input) {
        (Some(_), Some(piped)) => {
            eprintln!(
                "Warning: Both --description flag and piped input provided. Using piped input."
            );
            Some(piped)
        }
        (None, Some(piped)) => Some(piped),
        (Some(prompt), None) => Some(prompt),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::test_utils::TestGitRepository;

    #[test]
    fn test_merge_prompt_with_stdin() {
        // Test case 1: Both CLI and piped - piped wins
        let result =
            merge_prompt_with_stdin(Some("cli desc".to_string()), Some("piped desc".to_string()));
        assert_eq!(result, Some("piped desc".to_string()));

        // Test case 2: Only piped input
        let result = merge_prompt_with_stdin(None, Some("piped desc".to_string()));
        assert_eq!(result, Some("piped desc".to_string()));

        // Test case 3: Only CLI prompt
        let result = merge_prompt_with_stdin(Some("cli desc".to_string()), None);
        assert_eq!(result, Some("cli desc".to_string()));

        // Test case 4: No prompt
        let result = merge_prompt_with_stdin(None, None);
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
        let template_content = "# Task: {{TYPE}}\n{{PROMPT}}";
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

        // Create a template file without {{PROMPT}} placeholder
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

        // Should succeed for templates without {{PROMPT}} placeholder
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
