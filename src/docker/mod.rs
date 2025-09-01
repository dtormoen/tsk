pub mod composer;
pub mod image_manager;
pub mod layers;
pub mod proxy_manager;
pub mod template_engine;
pub mod template_manager;

use crate::agent::{Agent, LogProcessor};
use crate::context::docker_client::DockerClient;
use crate::docker::proxy_manager::ProxyManager;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{LogsOptions, RemoveContainerOptions};
use futures_util::stream::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Container resource limits
const CONTAINER_MEMORY_LIMIT: i64 = 4 * 1024 * 1024 * 1024; // 2GB
const CONTAINER_CPU_QUOTA: i64 = 400000; // 4 CPUs
const CONTAINER_WORKING_DIR: &str = "/workspace";
const CONTAINER_USER: &str = "agent";

pub struct DockerManager {
    client: Arc<dyn DockerClient>,
}

impl DockerManager {
    pub fn new(client: Arc<dyn DockerClient>) -> Self {
        Self { client }
    }

    fn prepare_worktree_path(worktree_path: &Path) -> Result<PathBuf, String> {
        // Convert to absolute path to ensure Docker can find the volume
        let absolute_path = if worktree_path.is_relative() {
            std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {e}"))?
                .join(worktree_path)
        } else {
            worktree_path.to_path_buf()
        };
        Ok(absolute_path)
    }

    /// Create a container configuration for both interactive and non-interactive modes.
    ///
    /// This function builds a unified container configuration that can be used for
    /// both interactive (TTY-attached) and non-interactive (log-streaming) containers.
    ///
    /// # Arguments
    /// * `image` - The Docker image to use
    /// * `worktree_path_str` - The absolute path to the work directory to mount
    /// * `command` - Optional command to run in the container
    /// * `interactive` - Whether to configure for interactive (TTY) mode
    /// * `instructions_file_path` - Optional path to instructions file to mount
    /// * `agent` - Optional agent to get volumes and environment variables from
    /// * `proxy_manager` - ProxyManager for network and proxy configuration
    ///
    /// # Returns
    /// A `ContainerCreateBody` configured with appropriate settings for the mode
    fn create_base_container_config(
        image: &str,
        worktree_path_str: &str,
        command: Option<Vec<String>>,
        interactive: bool,
        instructions_file_path: Option<&PathBuf>,
        agent: Option<&dyn Agent>,
        proxy_manager: &ProxyManager,
    ) -> ContainerCreateBody {
        // Build binds vector starting with workspace
        let mut binds = vec![format!("{worktree_path_str}:{CONTAINER_WORKING_DIR}")];

        // Add agent-specific volumes if provided
        if let Some(agent) = agent {
            for (host_path, container_path, options) in agent.volumes() {
                let bind = if options.is_empty() {
                    format!("{host_path}:{container_path}")
                } else {
                    format!("{host_path}:{container_path}:{options}")
                };
                binds.push(bind);
            }
        }

        // Add instructions directory mount if provided
        if let Some(inst_path) = instructions_file_path
            && let Some(parent) = inst_path.parent()
        {
            // Convert to absolute path to avoid Docker volume naming issues
            let abs_parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            binds.push(format!("{}:/instructions:ro", abs_parent.to_str().unwrap()));
        }

        // Build environment variables
        let mut env_vars = vec![
            // Configure proxy settings
            format!("HTTP_PROXY={}", proxy_manager.proxy_url()),
            format!("HTTPS_PROXY={}", proxy_manager.proxy_url()),
            format!("http_proxy={}", proxy_manager.proxy_url()),
            format!("https_proxy={}", proxy_manager.proxy_url()),
            // Don't proxy localhost
            "NO_PROXY=localhost,127.0.0.1".to_string(),
            "no_proxy=localhost,127.0.0.1".to_string(),
        ];

        // Add agent-specific environment variables if provided
        if let Some(agent) = agent {
            for (key, value) in agent.environment() {
                env_vars.push(format!("{key}={value}"));
            }
        } else {
            // Default environment if no agent specified
            env_vars.push(format!("HOME=/home/{CONTAINER_USER}"));
            env_vars.push(format!("USER={CONTAINER_USER}"));
        }

        ContainerCreateBody {
            image: Some(image.to_string()),
            // No entrypoint needed anymore - just run as agent user directly
            user: Some(CONTAINER_USER.to_string()),
            cmd: command,
            host_config: Some(HostConfig {
                binds: Some(binds),
                network_mode: Some(proxy_manager.network_name().to_string()),
                memory: Some(CONTAINER_MEMORY_LIMIT),
                cpu_quota: Some(CONTAINER_CPU_QUOTA),
                // No capabilities needed since we're not running iptables
                cap_drop: Some(vec![
                    "NET_ADMIN".to_string(),
                    "NET_RAW".to_string(),
                    "SETPCAP".to_string(),
                    "SYS_ADMIN".to_string(),
                    "SYS_PTRACE".to_string(),
                    "DAC_OVERRIDE".to_string(),
                    "AUDIT_WRITE".to_string(),
                    "SETUID".to_string(),
                    "SETGID".to_string(),
                ]),
                ..Default::default()
            }),
            working_dir: Some(CONTAINER_WORKING_DIR.to_string()),
            env: Some(env_vars),
            attach_stdin: Some(interactive),
            attach_stdout: Some(interactive),
            attach_stderr: Some(interactive),
            tty: Some(interactive),
            ..Default::default()
        }
    }

    /// Run a task container with unified support for both interactive and non-interactive modes.
    ///
    /// # Arguments
    /// * `image` - Docker image to use
    /// * `worktree_path` - Path to the work directory to mount
    /// * `instructions_file_path` - Optional path to instructions file
    /// * `agent` - The agent to use for the task
    /// * `is_interactive` - Whether to run in interactive mode
    /// * `task_id` - Task ID to use for container naming
    ///
    /// # Returns
    /// * `Ok((output, task_result))` - The container output and optional task result
    /// * `Err(String)` - Error message if container execution fails
    pub async fn run_task_container(
        &self,
        image: &str,
        worktree_path: &Path,
        instructions_file_path: Option<&PathBuf>,
        agent: &dyn Agent,
        is_interactive: bool,
        task_id: &str,
    ) -> Result<(String, Option<crate::agent::TaskResult>), String> {
        // Use ProxyManager to ensure proxy is running and healthy
        use crate::context::AppContext;
        let ctx = AppContext::builder()
            .with_docker_client(self.client.clone())
            .build();
        let proxy_manager = ProxyManager::new(&ctx);

        // Try to ensure proxy is running and healthy
        // If proxy fails to start or become healthy, return an error
        // This will cause the task to fail and can be retried later
        if let Err(e) = proxy_manager.ensure_proxy().await {
            return Err(format!(
                "Failed to ensure proxy is running and healthy: {e}. \
                The task should be retried later when the proxy is available. \
                Check the status in Docker."
            ));
        }

        let absolute_worktree_path = Self::prepare_worktree_path(worktree_path)?;
        let worktree_path_str = absolute_worktree_path
            .to_str()
            .ok_or_else(|| "Invalid worktree path".to_string())?;

        // Build the command based on whether we're interactive or not
        let command = if is_interactive {
            // For interactive mode, use the agent's interactive command
            let agent_command = agent.build_interactive_command(
                instructions_file_path
                    .and_then(|p| p.to_str())
                    .unwrap_or("instructions.md"),
            );
            if agent_command.is_empty() {
                None
            } else {
                Some(agent_command)
            }
        } else {
            // For non-interactive mode, just run the agent command
            let agent_command = agent.build_command(
                instructions_file_path
                    .and_then(|p| p.to_str())
                    .unwrap_or("instructions.md"),
            );
            if agent_command.is_empty() {
                None
            } else {
                Some(agent_command)
            }
        };

        // Create container configuration - shared for both modes
        let mut config = Self::create_base_container_config(
            image,
            worktree_path_str,
            command,
            is_interactive,
            instructions_file_path,
            Some(agent),
            &proxy_manager,
        );

        // Add open_stdin for interactive mode
        if is_interactive {
            config.open_stdin = Some(true);
        }

        // Create container name
        let container_name = if is_interactive {
            format!("tsk-interactive-{task_id}")
        } else {
            format!("tsk-{task_id}")
        };

        let options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        // Create the container
        let container_id = self.client.create_container(Some(options), config).await?;

        if is_interactive {
            // Interactive mode: start, attach, and cleanup
            println!("\nStarting interactive session...");
            self.client.start_container(&container_id).await?;

            // Attach to the container for interactive session
            let attach_result = self.client.attach_container(&container_id).await;

            // Clean up the container after the session ends
            let _ = self
                .client
                .remove_container(
                    &container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;

            if let Err(e) = attach_result {
                eprintln!("Interactive session ended with error: {e}");
                return Err(e);
            }

            println!("\nInteractive session ended");
            Ok((String::new(), None))
        } else {
            // Non-interactive mode: start, stream logs, process results
            println!("Starting agent sand box container: {container_id}");
            self.client.start_container(&container_id).await?;

            // Stream logs and process them
            let mut log_processor = agent.create_log_processor();
            let output = self
                .stream_container_logs(&container_id, &mut *log_processor)
                .await?;

            // Get the task result
            let task_result = log_processor.get_final_result().cloned();

            // Remove the container
            self.client
                .remove_container(
                    &container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await?;

            Ok((output, task_result))
        }
    }

    /// Stream container logs and process them through the log processor
    async fn stream_container_logs(
        &self,
        container_id: &str,
        log_processor: &mut dyn LogProcessor,
    ) -> Result<String, String> {
        // Start a background task to stream logs
        let client_clone = Arc::clone(&self.client);
        let container_id_clone = container_id.to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

        let log_task = tokio::spawn(async move {
            let log_options = LogsOptions {
                stdout: true,
                stderr: true,
                follow: true,
                timestamps: false,
                ..Default::default()
            };

            match client_clone
                .logs_stream(&container_id_clone, Some(log_options))
                .await
            {
                Ok(mut stream) => {
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(log_line) => {
                                if tx.send(log_line).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("Error streaming logs: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to start log streaming: {e}");
                }
            }
        });

        // Collect all logs for return value
        let mut all_logs = String::new();

        // Process logs while container is running
        loop {
            tokio::select! {
                Some(log_line) = rx.recv() => {
                    all_logs.push_str(&log_line);
                    // Process each line through the log processor
                    if let Some(formatted) = log_processor.process_line(&log_line) {
                        println!("{formatted}");
                    }
                }
                exit_code = self.client.wait_container(container_id) => {
                    let exit_code = exit_code?;

                    // Give a bit of time for remaining logs
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    // Drain any remaining logs
                    while let Ok(log_line) = rx.try_recv() {
                        all_logs.push_str(&log_line);
                        if let Some(formatted) = log_processor.process_line(&log_line) {
                            println!("{formatted}");
                        }
                    }

                    // Abort the log task
                    log_task.abort();

                    if exit_code == 0 {
                        return Ok(all_logs);
                    } else {
                        return Err(format!(
                            "Container exited with non-zero status: {exit_code}. Output:\n{all_logs}"
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::test_utils::TrackedDockerClient;

    #[tokio::test]
    async fn test_run_task_container_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");

        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        assert_eq!(output, "Container logs");
        assert!(task_result.is_none());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check that the command includes the agent command
        let task_container_config = &create_calls[1].1;
        let actual_cmd = task_container_config.cmd.as_ref().unwrap();
        // Command should be sh -c with the agent command
        assert_eq!(actual_cmd.len(), 3);
        assert_eq!(actual_cmd[0], "sh");
        assert_eq!(actual_cmd[1], "-c");
        assert!(actual_cmd[2].contains("claude"));

        // Check that user is set
        assert_eq!(task_container_config.user, Some("agent".to_string()));

        // Check proxy environment variables
        let env = task_container_config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));
        drop(create_calls); // Release the lock

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 2); // One for proxy, one for task container
        assert_eq!(start_calls[0], "tsk-proxy");
        assert_eq!(start_calls[1], "test-container-id-1");

        let wait_calls = mock_client.wait_container_calls.lock().unwrap();
        assert_eq!(wait_calls.len(), 1);
        assert_eq!(wait_calls[0], "test-container-id-1");

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id-1");
    }

    #[tokio::test]
    async fn test_run_task_container_interactive() {
        // Test interactive mode with mock client
        let mock_client = Arc::new(TrackedDockerClient::default());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");
        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);

        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                true, // interactive mode
                "test-task-id",
            )
            .await;

        // Interactive mode should succeed with mock client
        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        // Interactive mode returns empty output and no task result
        assert_eq!(output, "");
        assert!(task_result.is_none());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check task container config for interactive mode
        let (options, config) = &create_calls[1];
        assert_eq!(
            options.as_ref().unwrap().name,
            Some("tsk-interactive-test-task-id".to_string())
        );

        // Check interactive-specific settings
        assert_eq!(config.attach_stdin, Some(true));
        assert_eq!(config.attach_stdout, Some(true));
        assert_eq!(config.attach_stderr, Some(true));
        assert_eq!(config.tty, Some(true));
        assert_eq!(config.open_stdin, Some(true));

        // Verify attach_container was called (indirectly through start_container_calls)
        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 2); // One for proxy, one for task container

        // Verify cleanup happened
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1); // Task container should be removed
        assert_eq!(remove_calls[0].0, "test-container-id-1");
    }

    #[tokio::test]
    async fn test_run_task_container_non_zero_exit() {
        // Set up a mock client that will return a non-zero exit code
        let mock_client = Arc::new(TrackedDockerClient {
            exit_code: 1,
            ..Default::default()
        });
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");

        // Use AppContext builder to create test-safe directories and configs
        let app_context = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);

        // Run the task container
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        // With the new behavior, the container logs are returned even with non-zero exit
        // The error handling is now done by the agent's log processor
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("Container exited with non-zero status: 1"));
        assert!(error_msg.contains("Container logs")); // Should include the output

        // Note: Currently, when stream_container_logs returns an error, the container
        // is not removed. This could be improved to ensure cleanup always happens.
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_run_task_container_create_fails() {
        let mock_client = TrackedDockerClient {
            network_exists: false,
            create_network_error: Some("Docker daemon not running".to_string()),
            ..Default::default()
        };
        let mock_client = Arc::new(mock_client);
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");

        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        // The error message should indicate proxy startup failure
        assert!(error_msg.contains("Failed to ensure proxy is running and healthy"));
        // The error chain should include the network creation failure
        assert!(
            error_msg.contains("Failed to ensure network exists")
                || error_msg.contains("Failed to create network")
        );

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_container_configuration() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");

        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);
        let _ = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check proxy container config (proxy manager creates this)
        let (proxy_options, _proxy_config) = &create_calls[0];
        assert_eq!(
            proxy_options.as_ref().unwrap().name,
            Some("tsk-proxy".to_string())
        );

        // Check task container config
        let (options, config) = &create_calls[1];

        assert!(
            options
                .as_ref()
                .unwrap()
                .name
                .as_ref()
                .unwrap()
                .starts_with("tsk-")
        );
        assert_eq!(
            options.as_ref().unwrap().name,
            Some("tsk-test-task-id".to_string())
        );
        assert_eq!(config.image, Some("tsk/base".to_string()));
        assert_eq!(config.working_dir, Some(CONTAINER_WORKING_DIR.to_string()));
        // User is now set directly
        assert_eq!(config.user, Some(CONTAINER_USER.to_string()));

        // Check that command includes the agent command
        let actual_cmd = config.cmd.as_ref().unwrap();
        assert_eq!(actual_cmd.len(), 3);
        assert_eq!(actual_cmd[0], "sh");
        assert_eq!(actual_cmd[1], "-c");
        assert!(actual_cmd[2].contains("claude"));

        // No entrypoint anymore
        assert!(config.entrypoint.is_none());

        let host_config = config.host_config.as_ref().unwrap();
        assert_eq!(host_config.network_mode, Some("tsk-network".to_string()));
        assert_eq!(host_config.memory, Some(CONTAINER_MEMORY_LIMIT));
        assert_eq!(host_config.cpu_quota, Some(CONTAINER_CPU_QUOTA));

        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 3);
        assert!(binds[0].contains(&format!("/tmp/test-worktree:{CONTAINER_WORKING_DIR}")));
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/home/agent/.claude.json"));

        // Check proxy environment variables
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));
    }

    #[tokio::test]
    async fn test_run_task_container_with_instructions_file() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        let worktree_path = Path::new("/tmp/test-worktree");
        let instructions_path = PathBuf::from("/tmp/tsk-test/instructions.txt");

        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                Some(&instructions_path),
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check that instructions directory is mounted
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, claude.json, and instructions
        assert!(binds[3].contains("/tmp/tsk-test:/instructions:ro"));
    }

    #[tokio::test]
    async fn test_relative_path_conversion() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>);

        // Create a temporary directory to use as base
        let temp_dir = tempfile::TempDir::new().unwrap();
        let absolute_path = temp_dir.path().join("test-worktree");

        let app_context = AppContext::builder().build();
        let tsk_config = app_context.tsk_config();
        let agent = crate::agent::ClaudeCodeAgent::with_tsk_config(tsk_config);
        let result = manager
            .run_task_container(
                "tsk/base",
                &absolute_path,
                None,
                &agent,
                false, // not interactive
                "test-task-id",
            )
            .await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container
        let (_, config) = &create_calls[1]; // Get the task container config, not proxy

        let host_config = config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        let worktree_bind = &binds[0];

        // Should contain an absolute path (starts with /)
        assert!(worktree_bind.starts_with('/'));
        assert!(worktree_bind.contains("test-worktree"));
        assert!(worktree_bind.ends_with(&format!(":{CONTAINER_WORKING_DIR}")));

        // Should also have the claude directory and claude.json mounts
        assert_eq!(binds.len(), 3);
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/home/agent/.claude.json"));
    }
}
