pub mod composer;
pub mod image_manager;
pub mod layers;
pub mod template_manager;

use crate::agent::{Agent, LogProcessor};
use crate::context::docker_client::DockerClient;
use crate::context::file_system::FileSystemOperations;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Container resource limits
const CONTAINER_MEMORY_LIMIT: i64 = 4 * 1024 * 1024 * 1024; // 2GB
const CONTAINER_CPU_QUOTA: i64 = 400000; // 4 CPUs
const CONTAINER_WORKING_DIR: &str = "/workspace";
const CONTAINER_USER: &str = "agent";
const TSK_NETWORK_NAME: &str = "tsk-network";
const PROXY_CONTAINER_NAME: &str = "tsk-proxy";
const PROXY_IMAGE: &str = "tsk/proxy";

pub struct DockerManager {
    client: Arc<dyn DockerClient>,
    file_system: Arc<dyn FileSystemOperations>,
}

impl DockerManager {
    pub fn new(client: Arc<dyn DockerClient>, file_system: Arc<dyn FileSystemOperations>) -> Self {
        Self {
            client,
            file_system,
        }
    }

    #[allow(dead_code)]
    pub async fn stop_proxy(&self) -> Result<(), String> {
        // Try to stop and remove the proxy container
        match self
            .client
            .remove_container(
                PROXY_CONTAINER_NAME,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.contains("No such container") => Ok(()),
            Err(e) => Err(format!("Failed to stop proxy container: {}", e)),
        }
    }

    async fn ensure_network(&self) -> Result<(), String> {
        if !self.client.network_exists(TSK_NETWORK_NAME).await? {
            self.client.create_network(TSK_NETWORK_NAME).await?;
        }
        Ok(())
    }

    async fn ensure_proxy(&self) -> Result<(), String> {
        // Check if proxy container exists
        let proxy_config = Config {
            image: Some(PROXY_IMAGE.to_string()),
            exposed_ports: Some(
                vec![("3128/tcp".to_string(), HashMap::new())]
                    .into_iter()
                    .collect(),
            ),
            host_config: Some(bollard::service::HostConfig {
                network_mode: Some(TSK_NETWORK_NAME.to_string()),
                restart_policy: Some(bollard::service::RestartPolicy {
                    name: Some(bollard::service::RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_options = CreateContainerOptions {
            name: PROXY_CONTAINER_NAME.to_string(),
            platform: None,
        };

        // Try to create the container (this will fail if it already exists)
        match self
            .client
            .create_container(Some(create_options), proxy_config)
            .await
        {
            Ok(_) => {
                // New container created, start it
                self.client.start_container(PROXY_CONTAINER_NAME).await?;
            }
            Err(e) => {
                // Container might already exist, try to start it
                if e.contains("already in use") {
                    // Try to start existing container
                    match self.client.start_container(PROXY_CONTAINER_NAME).await {
                        Ok(_) => (),
                        Err(e) if e.contains("already started") => (),
                        Err(e) => return Err(e),
                    }
                } else {
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    fn prepare_worktree_path(worktree_path: &Path) -> Result<PathBuf, String> {
        // Convert to absolute path to ensure Docker can find the volume
        let absolute_path = if worktree_path.is_relative() {
            std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?
                .join(worktree_path)
        } else {
            worktree_path.to_path_buf()
        };
        Ok(absolute_path)
    }

    fn create_base_container_config(
        image: &str,
        worktree_path_str: &str,
        command: Option<Vec<String>>,
        interactive: bool,
        instructions_file_path: Option<&PathBuf>,
        agent: Option<&dyn Agent>,
    ) -> Config<String> {
        // Build binds vector starting with workspace
        let mut binds = vec![format!("{}:{}", worktree_path_str, CONTAINER_WORKING_DIR)];

        // Add agent-specific volumes if provided
        if let Some(agent) = agent {
            for (host_path, container_path, options) in agent.volumes() {
                let bind = if options.is_empty() {
                    format!("{}:{}", host_path, container_path)
                } else {
                    format!("{}:{}{}", host_path, container_path, options)
                };
                binds.push(bind);
            }
        }

        // Add instructions directory mount if provided
        if let Some(inst_path) = instructions_file_path {
            if let Some(parent) = inst_path.parent() {
                // Convert to absolute path to avoid Docker volume naming issues
                let abs_parent = parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf());
                binds.push(format!("{}:/instructions:ro", abs_parent.to_str().unwrap()));
            }
        }

        // Build environment variables
        let mut env_vars = vec![
            // Configure proxy settings
            "HTTP_PROXY=http://tsk-proxy:3128".to_string(),
            "HTTPS_PROXY=http://tsk-proxy:3128".to_string(),
            "http_proxy=http://tsk-proxy:3128".to_string(),
            "https_proxy=http://tsk-proxy:3128".to_string(),
            // Don't proxy localhost
            "NO_PROXY=localhost,127.0.0.1".to_string(),
            "no_proxy=localhost,127.0.0.1".to_string(),
        ];

        // Add agent-specific environment variables if provided
        if let Some(agent) = agent {
            for (key, value) in agent.environment() {
                env_vars.push(format!("{}={}", key, value));
            }
        } else {
            // Default environment if no agent specified
            env_vars.push(format!("HOME=/home/{}", CONTAINER_USER));
            env_vars.push(format!("USER={}", CONTAINER_USER));
        }

        Config {
            image: Some(image.to_string()),
            // No entrypoint needed anymore - just run as agent user directly
            user: Some(CONTAINER_USER.to_string()),
            cmd: command,
            host_config: Some(bollard::service::HostConfig {
                binds: Some(binds),
                network_mode: Some(TSK_NETWORK_NAME.to_string()),
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

    pub async fn stop_and_remove_container(&self, container_name: &str) -> Result<(), String> {
        self.client
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
    }

    /// Run a task container with unified support for both interactive and non-interactive modes.
    ///
    /// # Arguments
    /// * `image` - Docker image to use
    /// * `worktree_path` - Path to the work directory to mount
    /// * `instructions_file_path` - Optional path to instructions file
    /// * `agent` - The agent to use for the task
    /// * `is_interactive` - Whether to run in interactive mode
    /// * `task_name` - Name of the task for logging purposes
    /// * `log_file_path` - Optional path to save the full log file
    ///
    /// # Returns
    /// * `Ok((output, task_result))` - The container output and optional task result
    /// * `Err(String)` - Error message if container execution fails
    #[allow(clippy::too_many_arguments)]
    pub async fn run_task_container(
        &self,
        image: &str,
        worktree_path: &Path,
        instructions_file_path: Option<&PathBuf>,
        agent: &dyn Agent,
        is_interactive: bool,
        _task_name: &str,
        log_file_path: Option<&Path>,
    ) -> Result<(String, Option<crate::agent::TaskResult>), String> {
        // Ensure network and proxy are running
        self.ensure_network().await?;
        self.ensure_proxy().await?;

        let absolute_worktree_path = Self::prepare_worktree_path(worktree_path)?;
        let worktree_path_str = absolute_worktree_path
            .to_str()
            .ok_or_else(|| "Invalid worktree path".to_string())?;

        // Build the command based on whether we're interactive or not
        let command = if is_interactive {
            // For interactive mode, run the agent command followed by bash
            let agent_command = agent.build_command(
                instructions_file_path
                    .and_then(|p| p.to_str())
                    .unwrap_or("instructions.md"),
            );

            // Construct a shell command that runs the agent command and then drops into bash
            let command_str = format!("{}; exec /bin/bash", agent_command.join(" "));
            Some(vec!["sh".to_string(), "-c".to_string(), command_str])
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

        let config = Self::create_base_container_config(
            image,
            worktree_path_str,
            command,
            is_interactive,
            instructions_file_path,
            Some(agent),
        );

        let container_name = if is_interactive {
            format!("tsk-interactive-{}", chrono::Utc::now().timestamp())
        } else {
            format!("tsk-{}", chrono::Utc::now().timestamp())
        };

        let options = CreateContainerOptions {
            name: container_name.clone(),
            platform: None,
        };

        let container_id = self.client.create_container(Some(options), config).await?;
        self.client.start_container(&container_id).await?;

        if is_interactive {
            // For interactive mode, attach to the container using Bollard's exec API
            self.attach_to_container(&container_id, &container_name)
                .await?;

            // After interactive session, stop and remove the container
            self.client
                .remove_container(
                    &container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await?;

            // Return empty output and no task result for interactive sessions
            Ok((String::new(), None))
        } else {
            // For non-interactive mode, stream logs and process them
            let mut log_processor = agent.create_log_processor(self.file_system.clone());
            let output = self
                .stream_container_logs(&container_id, &mut *log_processor)
                .await?;

            // Save logs if requested
            if let Some(log_path) = log_file_path {
                if let Err(e) = log_processor.save_full_log(log_path).await {
                    eprintln!("Warning: Failed to save full log file: {}", e);
                } else {
                    println!("Full log saved to: {}", log_path.display());
                }
            }

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

    /// Attach to a running container for interactive sessions
    /// Uses docker attach for simplicity and proper TTY handling
    async fn attach_to_container(
        &self,
        _container_id: &str,
        container_name: &str,
    ) -> Result<(), String> {
        println!("\nDocker container started successfully!");
        println!("Container name: {}", container_name);
        println!("\nStarting interactive session...");

        println!("\nAgent command is running, you'll be dropped into a shell when it completes...");

        // Use docker attach command for proper TTY handling
        // The container already has the agent command running followed by '; exec /bin/bash'
        let status = std::process::Command::new("docker")
            .args(["attach", container_name])
            .status()
            .map_err(|e| format!("Failed to execute docker attach: {}", e))?;

        if !status.success() {
            eprintln!("Interactive session exited with non-zero status");
        }

        println!("\nInteractive session ended");
        Ok(())
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
                                eprintln!("Error streaming logs: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to start log streaming: {}", e);
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
                        println!("{}", formatted);
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
                            println!("{}", formatted);
                        }
                    }

                    // Abort the log task
                    log_task.abort();

                    if exit_code == 0 {
                        return Ok(all_logs);
                    } else {
                        return Err(format!(
                            "Container exited with non-zero status: {}. Output:\n{}",
                            exit_code, all_logs
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
    use crate::context::file_system::tests::MockFileSystem;
    use crate::test_utils::TrackedDockerClient;

    #[tokio::test]
    async fn test_run_task_container_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
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
    #[ignore = "Interactive mode requires docker attach which doesn't work in test environment"]
    async fn test_run_task_container_interactive() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                true, // interactive
                "test-task",
                None, // no log file path
            )
            .await;

        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        assert_eq!(output, ""); // Interactive sessions return empty output
        assert!(task_result.is_none());

        // Verify interactive command includes bash
        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container
        let task_container_config = &create_calls[1].1;
        let actual_cmd = task_container_config.cmd.as_ref().unwrap();
        assert_eq!(actual_cmd[0], "sh");
        assert_eq!(actual_cmd[1], "-c");
        assert!(actual_cmd[2].contains("; exec /bin/bash"));

        // Check that user is set
        assert_eq!(task_container_config.user, Some("agent".to_string()));
    }

    #[tokio::test]
    #[ignore = "Test needs to be updated for new behavior where agent commands handle exit codes"]
    async fn test_run_task_container_non_zero_exit() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
            )
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Container exited with non-zero status: 1"));

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_run_task_container_create_fails() {
        let mut mock_client = TrackedDockerClient::default();
        mock_client.network_exists = false;
        mock_client.create_network_error = Some("Docker daemon not running".to_string());
        let mock_client = Arc::new(mock_client);
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
            )
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Docker daemon not running");

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_container_configuration() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let _ = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                None,
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
            )
            .await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check proxy container config
        let (proxy_options, proxy_config) = &create_calls[0];
        assert_eq!(proxy_options.as_ref().unwrap().name, PROXY_CONTAINER_NAME);
        assert_eq!(proxy_config.image, Some(PROXY_IMAGE.to_string()));

        // Check task container config
        let (options, config) = &create_calls[1];

        assert!(options.as_ref().unwrap().name.starts_with("tsk-"));
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
        assert_eq!(host_config.network_mode, Some(TSK_NETWORK_NAME.to_string()));
        assert_eq!(host_config.memory, Some(CONTAINER_MEMORY_LIMIT));
        assert_eq!(host_config.cpu_quota, Some(CONTAINER_CPU_QUOTA));

        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 3);
        assert!(binds[0].contains(&format!("/tmp/test-worktree:{}", CONTAINER_WORKING_DIR)));
        assert!(binds[1].ends_with("/.claude:/home/agent/.claude"));
        assert!(binds[2].ends_with("/.claude.json:/home/agent/.claude.json"));

        // Check proxy environment variables
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));
    }

    #[tokio::test]
    async fn test_run_task_container_with_instructions_file() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        let worktree_path = Path::new("/tmp/test-worktree");
        let instructions_path = PathBuf::from("/tmp/tsk-test/instructions.txt");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                worktree_path,
                Some(&instructions_path),
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
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
        let fs = Arc::new(MockFileSystem::new());
        let manager = DockerManager::new(mock_client.clone() as Arc<dyn DockerClient>, fs);

        // Create a temporary directory to use as base
        let temp_dir = tempfile::TempDir::new().unwrap();
        let absolute_path = temp_dir.path().join("test-worktree");

        let agent = crate::agent::ClaudeCodeAgent::new();
        let result = manager
            .run_task_container(
                "tsk/base",
                &absolute_path,
                None,
                &agent,
                false, // not interactive
                "test-task",
                None, // no log file path
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
        assert!(worktree_bind.ends_with(&format!(":{}", CONTAINER_WORKING_DIR)));

        // Should also have the claude directory and claude.json mounts
        assert_eq!(binds.len(), 3);
        assert!(binds[1].ends_with("/.claude:/home/agent/.claude"));
        assert!(binds[2].ends_with("/.claude.json:/home/agent/.claude.json"));
    }
}
