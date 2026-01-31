pub mod build_lock_manager;
pub mod composer;
pub mod image_manager;
pub mod layers;
pub mod proxy_manager;
pub mod template_engine;
pub mod template_manager;

use crate::agent::{Agent, LogProcessor};
use crate::context::AppContext;
use crate::context::VolumeMount;
use crate::docker::proxy_manager::ProxyManager;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{LogsOptions, RemoveContainerOptions};
use futures_util::stream::StreamExt;
use std::path::PathBuf;

const CONTAINER_WORKING_DIR: &str = "/workspace";
const CONTAINER_USER: &str = "agent";

/// Manages Docker container execution for TSK tasks.
///
/// This struct handles the lifecycle of task containers including:
/// - Container configuration and creation
/// - Proxy management for network isolation
/// - Log streaming and processing
/// - Container cleanup
pub struct DockerManager {
    ctx: AppContext,
    proxy_manager: ProxyManager,
}

impl DockerManager {
    /// Creates a new DockerManager with the given application context.
    ///
    /// # Arguments
    /// * `ctx` - The application context containing all dependencies
    pub fn new(ctx: &AppContext) -> Self {
        let proxy_manager = ProxyManager::new(ctx);
        Self {
            ctx: ctx.clone(),
            proxy_manager,
        }
    }

    /// Build proxy environment variables
    fn build_proxy_env_vars(&self) -> Vec<String> {
        let proxy_url = self.proxy_manager.proxy_url();
        vec![
            format!("HTTP_PROXY={proxy_url}"),
            format!("HTTPS_PROXY={proxy_url}"),
            format!("http_proxy={proxy_url}"),
            format!("https_proxy={proxy_url}"),
            "NO_PROXY=localhost,127.0.0.1,host.docker.internal".to_string(),
            "no_proxy=localhost,127.0.0.1,host.docker.internal".to_string(),
        ]
    }

    /// Remove a container with force option
    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        self.ctx
            .docker_client()
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| e.to_string())
    }

    /// Build bind volumes for container
    fn build_bind_volumes(&self, task: &crate::task::Task, agent: &dyn Agent) -> Vec<String> {
        let repo_path_str = task
            .copied_repo_path
            .to_str()
            .expect("Repository path should be valid UTF-8");
        let mut binds = vec![format!("{repo_path_str}:{CONTAINER_WORKING_DIR}")];

        // Add agent-specific volumes
        for (host_path, container_path, options) in agent.volumes() {
            let bind = if options.is_empty() {
                format!("{host_path}:{container_path}")
            } else {
                format!("{host_path}:{container_path}:{options}")
            };
            binds.push(bind);
        }

        // Add instructions directory mount
        let instructions_file_path = PathBuf::from(&task.instructions_file);
        if let Some(parent) = instructions_file_path.parent() {
            let abs_parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            binds.push(format!("{}:/instructions:ro", abs_parent.to_string_lossy()));
        }

        // Add output directory mount
        if let Some(task_dir) = task.copied_repo_path.parent() {
            let output_dir = task_dir.join("output");
            binds.push(format!("{}:/output", output_dir.to_string_lossy()));
        }

        // Add project-specific volume mounts from config
        if let Some(project_config) = self.ctx.tsk_config().get_project_config(&task.project) {
            for volume in &project_config.volumes {
                match volume {
                    VolumeMount::Bind(bind) => {
                        if let Ok(host_path) = bind.expanded_host_path() {
                            let bind_str = if bind.readonly {
                                format!("{}:{}:ro", host_path.display(), bind.container)
                            } else {
                                format!("{}:{}", host_path.display(), bind.container)
                            };
                            binds.push(bind_str);
                        }
                    }
                    VolumeMount::Named(named) => {
                        let volume_name = format!("tsk-{}", named.name);
                        let bind_str = if named.readonly {
                            format!("{volume_name}:{}:ro", named.container)
                        } else {
                            format!("{volume_name}:{}", named.container)
                        };
                        binds.push(bind_str);
                    }
                }
            }
        }

        binds
    }

    /// Generate container name based on task mode
    fn build_container_name(&self, task: &crate::task::Task) -> String {
        if task.is_interactive {
            format!("tsk-interactive-{}", task.id)
        } else {
            format!("tsk-{}", task.id)
        }
    }

    /// Create a container configuration for both interactive and non-interactive modes.
    ///
    /// This function builds a unified container configuration that can be used for
    /// both interactive (TTY-attached) and non-interactive (log-streaming) containers.
    ///
    /// # Arguments
    /// * `image` - The Docker image to use
    /// * `task` - The task containing all necessary configuration
    /// * `agent` - The agent to get volumes and environment variables from
    /// * `network_name` - The Docker network name for the container to join
    ///
    /// # Returns
    /// A `ContainerCreateBody` configured with appropriate settings for the mode
    fn create_container_config(
        &self,
        image: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
        network_name: &str,
    ) -> ContainerCreateBody {
        let binds = self.build_bind_volumes(task, agent);
        let instructions_file_path = PathBuf::from(&task.instructions_file);
        let mut env_vars = self.build_proxy_env_vars();

        // Add agent-specific environment variables
        for (key, value) in agent.environment() {
            env_vars.push(format!("{key}={value}"));
        }

        let agent_command = agent.build_command(
            instructions_file_path.to_str().unwrap_or("instructions.md"),
            task.is_interactive,
        );

        let command = if agent_command.is_empty() {
            None
        } else {
            Some(agent_command)
        };

        // Load Docker resource limits from user configuration
        let docker_config = &self.ctx.tsk_config().docker;

        ContainerCreateBody {
            image: Some(image.to_string()),
            // No entrypoint needed anymore - just run as agent user directly
            user: Some(CONTAINER_USER.to_string()),
            cmd: command,
            host_config: Some(HostConfig {
                binds: Some(binds),
                network_mode: Some(network_name.to_string()),
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
                memory: Some(docker_config.memory_limit_bytes()),
                cpu_quota: Some(docker_config.cpu_quota_microseconds()),
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
            attach_stdin: Some(task.is_interactive),
            attach_stdout: Some(task.is_interactive),
            attach_stderr: Some(task.is_interactive),
            tty: Some(task.is_interactive),
            open_stdin: Some(task.is_interactive),
            ..Default::default()
        }
    }

    /// Run a task container with unified support for both interactive and non-interactive modes.
    ///
    /// # Arguments
    /// * `docker_image_tag` - Docker image tag to use
    /// * `task` - The task to execute
    /// * `agent` - The agent to use for the task
    ///
    /// # Returns
    /// * `Ok((output, task_result))` - The container output and optional task result
    /// * `Err(String)` - Error message if container execution fails
    pub async fn run_task_container(
        &self,
        docker_image_tag: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
    ) -> Result<(String, Option<crate::agent::TaskResult>), String> {
        // Try to ensure proxy is running and healthy
        // If proxy fails to start or become healthy, return an error
        // This will cause the task to fail and can be retried later
        if let Err(e) = self.proxy_manager.ensure_proxy().await {
            return Err(format!(
                "Failed to ensure proxy is running and healthy: {e}. \
                The task should be retried later when the proxy is available. \
                Check the status in Docker."
            ));
        }

        // Create isolated network for this agent
        let network_name = self
            .proxy_manager
            .create_agent_network(&task.id)
            .await
            .map_err(|e| format!("Failed to create agent network: {e}"))?;

        // Connect proxy to the agent's network before starting container
        if let Err(e) = self
            .proxy_manager
            .connect_proxy_to_network(&network_name)
            .await
        {
            // Clean up network on failure
            self.proxy_manager
                .cleanup_agent_network(&network_name)
                .await;
            return Err(format!("Failed to connect proxy to agent network: {e}"));
        }

        let config = self.create_container_config(docker_image_tag, task, agent, &network_name);
        let container_name = self.build_container_name(task);
        let options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        let container_id = match self
            .ctx
            .docker_client()
            .create_container(Some(options), config)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                // Clean up network on failure
                self.proxy_manager
                    .cleanup_agent_network(&network_name)
                    .await;
                return Err(e);
            }
        };

        // Copy agent files into container before starting
        for (tar_data, dest_path) in agent.files_to_copy() {
            self.ctx
                .docker_client()
                .upload_to_container(&container_id, &dest_path, tar_data)
                .await?;
        }

        let result = if task.is_interactive {
            println!("\nStarting interactive session...");
            if let Err(e) = self
                .ctx
                .docker_client()
                .start_container(&container_id)
                .await
            {
                let _ = self.remove_container(&container_id).await;
                self.proxy_manager
                    .cleanup_agent_network(&network_name)
                    .await;
                return Err(e);
            }

            let attach_result = self
                .ctx
                .docker_client()
                .attach_container(&container_id)
                .await;

            let _ = self.remove_container(&container_id).await;
            self.proxy_manager
                .cleanup_agent_network(&network_name)
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
            if let Err(e) = self
                .ctx
                .docker_client()
                .start_container(&container_id)
                .await
            {
                let _ = self.remove_container(&container_id).await;
                self.proxy_manager
                    .cleanup_agent_network(&network_name)
                    .await;
                return Err(e);
            }

            // Stream logs and process them
            let mut log_processor = agent.create_log_processor(Some(task));
            let output = self
                .stream_container_logs(&container_id, &mut *log_processor)
                .await;

            let task_result = log_processor.get_final_result().cloned();

            let _ = self.remove_container(&container_id).await;
            self.proxy_manager
                .cleanup_agent_network(&network_name)
                .await;

            match output {
                Ok(output) => Ok((output, task_result)),
                Err(e) => Err(e),
            }
        };

        // After task completion (success or failure), try to stop proxy if idle
        // This handles single-run mode (tsk run, tsk shell)
        // Errors during proxy cleanup are logged but don't affect the task result
        if let Err(e) = self.proxy_manager.maybe_stop_proxy().await {
            eprintln!("Warning: Failed to check/stop idle proxy: {e}");
        }

        result
    }

    /// Stream container logs and process them through the log processor
    async fn stream_container_logs(
        &self,
        container_id: &str,
        log_processor: &mut dyn LogProcessor,
    ) -> Result<String, String> {
        // Start a background task to stream logs
        let client_clone = self.ctx.docker_client();
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

        // Buffer for accumulating partial lines from Docker chunks
        let mut line_buffer = String::new();

        // Get docker client to avoid temporary value issues
        let docker_client = self.ctx.docker_client();

        // Create the wait future ONCE and pin it so it persists across loop iterations.
        // This is critical - tokio::select! in a loop drops unselected futures each iteration,
        // so without pinning, wait_container would be called multiple times.
        let wait_future = docker_client.wait_container(container_id);
        tokio::pin!(wait_future);

        // Process logs while container is running
        loop {
            tokio::select! {
                Some(log_chunk) = rx.recv() => {
                    // Keep raw chunks in all_logs for full log capture
                    all_logs.push_str(&log_chunk);

                    // Buffer chunks and process complete lines only
                    line_buffer.push_str(&log_chunk);
                    process_complete_lines(&mut line_buffer, log_processor);
                }
                exit_code = &mut wait_future => {
                    let exit_code = exit_code?;

                    // Give a bit of time for remaining logs
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    // Drain any remaining logs
                    while let Ok(log_chunk) = rx.try_recv() {
                        all_logs.push_str(&log_chunk);
                        line_buffer.push_str(&log_chunk);
                        process_complete_lines(&mut line_buffer, log_processor);
                    }

                    // Flush remaining buffer content if non-empty
                    if !line_buffer.trim().is_empty() {
                        let trimmed = line_buffer.trim_end_matches('\r');
                        if let Some(formatted) = log_processor.process_line(trimmed) {
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

/// Process complete lines from the buffer and pass them to the log processor.
///
/// This function extracts all complete lines (terminated by newline) from the buffer,
/// processes each through the log processor, and removes them from the buffer.
/// Any partial line (without a trailing newline) remains in the buffer.
fn process_complete_lines(line_buffer: &mut String, log_processor: &mut dyn LogProcessor) {
    while let Some(newline_pos) = line_buffer.find('\n') {
        let complete_line = &line_buffer[..newline_pos];
        // Handle CRLF by trimming trailing \r
        let trimmed = complete_line.trim_end_matches('\r');

        if let Some(formatted) = log_processor.process_line(trimmed) {
            println!("{formatted}");
        }

        // Use drain() for efficient in-place removal
        line_buffer.drain(..=newline_pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::TrackedDockerClient;
    use std::sync::Arc;

    fn create_test_task(is_interactive: bool) -> Task {
        let repo_path = PathBuf::from("/tmp/test-repo");
        Task {
            id: "test-task-id".to_string(),
            repo_root: repo_path.clone(),
            name: "test-task".to_string(),
            task_type: "feature".to_string(),
            instructions_file: "/tmp/test-repo/.tsk/tasks/instructions.md".to_string(),
            agent: "claude".to_string(),
            status: TaskStatus::Running,
            created_at: chrono::Local::now(),
            started_at: Some(chrono::Utc::now()),
            completed_at: None,
            branch_name: "tsk/feature/test-task/test-task-id".to_string(),
            error_message: None,
            source_commit: "abc123".to_string(),
            source_branch: Some("main".to_string()),
            stack: "default".to_string(),
            project: "default".to_string(),
            copied_repo_path: repo_path,
            is_interactive,
        }
    }

    #[tokio::test]
    async fn test_run_task_container_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

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
        drop(remove_calls);

        // Verify upload_to_container was called for agent files
        // Note: In tests, claude.json may or may not exist, so we just verify it was called
        // if the agent has files to copy
        let upload_calls = mock_client.upload_to_container_calls.lock().unwrap();
        // The number of calls depends on whether .claude.json exists in test environment
        // For this test, just verify the method was callable
        for (container_id, dest_path, _tar_data) in upload_calls.iter() {
            assert_eq!(container_id, "test-container-id-1");
            assert_eq!(dest_path, "/home/agent");
        }
    }

    #[tokio::test]
    async fn test_run_task_container_interactive() {
        // Test interactive mode with mock client
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(true);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

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
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());

        // Run the task container
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        // With the new behavior, the container logs are returned even with non-zero exit
        // The error handling is now done by the agent's log processor
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("Container exited with non-zero status: 1"));
        assert!(error_msg.contains("Container logs")); // Should include the output

        // Cleanup now always happens even on error
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id-1");
        drop(remove_calls);

        // Network cleanup should also happen
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_run_task_container_create_fails() {
        let mock_client = TrackedDockerClient {
            network_exists: false,
            create_network_error: Some("Docker daemon not running".to_string()),
            ..Default::default()
        };
        let mock_client = Arc::new(mock_client);
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

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
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

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
        // Network mode should use task-specific isolated network
        assert_eq!(
            host_config.network_mode,
            Some("tsk-agent-test-task-id".to_string())
        );
        // extra_hosts should include host.docker.internal for host service access
        let extra_hosts = host_config.extra_hosts.as_ref().unwrap();
        assert!(extra_hosts.contains(&"host.docker.internal:host-gateway".to_string()));
        let default_options = crate::context::DockerOptions::default();
        assert_eq!(
            host_config.memory,
            Some(default_options.memory_limit_bytes())
        );
        assert_eq!(
            host_config.cpu_quota,
            Some(default_options.cpu_quota_microseconds())
        );

        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        assert!(binds[0].contains(&format!("/tmp/test-repo:{CONTAINER_WORKING_DIR}")));
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/instructions:ro"));
        assert!(binds[3].contains(":/output"));

        // Check proxy environment variables
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));
        // NO_PROXY should include host.docker.internal for host service access
        assert!(env.contains(&"NO_PROXY=localhost,127.0.0.1,host.docker.internal".to_string()));
        assert!(env.contains(&"no_proxy=localhost,127.0.0.1,host.docker.internal".to_string()));
        drop(create_calls);

        // Verify network lifecycle operations were called
        let create_internal_network_calls =
            mock_client.create_internal_network_calls.lock().unwrap();
        assert_eq!(create_internal_network_calls.len(), 1);
        assert_eq!(create_internal_network_calls[0], "tsk-agent-test-task-id");
        drop(create_internal_network_calls);

        let connect_calls = mock_client.connect_network_calls.lock().unwrap();
        assert_eq!(connect_calls.len(), 1);
        assert_eq!(
            connect_calls[0],
            (
                "tsk-proxy".to_string(),
                "tsk-agent-test-task-id".to_string()
            )
        );
        drop(connect_calls);

        // Verify network cleanup was called
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        assert_eq!(
            disconnect_calls[0],
            (
                "tsk-proxy".to_string(),
                "tsk-agent-test-task-id".to_string()
            )
        );
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
        assert_eq!(remove_network_calls[0], "tsk-agent-test-task-id");
    }

    #[tokio::test]
    async fn test_run_task_container_with_instructions_file() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.instructions_file = "/tmp/tsk-test/instructions.txt".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check that instructions directory is mounted
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        assert!(binds[2].contains("/tmp/tsk-test:/instructions:ro"));
        assert!(binds[3].contains(":/output"));
    }

    #[tokio::test]
    async fn test_relative_path_conversion() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        // Create a temporary directory to use as base
        let temp_dir = tempfile::TempDir::new().unwrap();
        let absolute_path = temp_dir.path().join("test-repo");

        let mut task = create_test_task(false);
        task.copied_repo_path = absolute_path.clone();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container
        let (_, config) = &create_calls[1]; // Get the task container config, not proxy

        let host_config = config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        let repo_bind = &binds[0];

        // Should contain an absolute path (starts with /)
        assert!(repo_bind.starts_with('/'));
        assert!(repo_bind.contains("test-repo"));
        assert!(repo_bind.ends_with(&format!(":{CONTAINER_WORKING_DIR}")));

        // Should also have the claude directory, instructions, and output mounts
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/instructions:ro"));
        assert!(binds[3].contains(":/output"));
    }

    #[tokio::test]
    async fn test_project_volume_mounts_bind() {
        use crate::context::{BindMount, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a bind mount for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/host/cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
            },
        );
        let tsk_config = TskConfig {
            docker: Default::default(),
            git_town: Default::default(),
            project: project_configs,
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + project bind mount
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("/host/cache:/container/cache"))
        );
    }

    #[tokio::test]
    async fn test_project_volume_mounts_named() {
        use crate::context::{NamedVolume, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a named volume for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Named(NamedVolume {
                    name: "build-cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
            },
        );
        let tsk_config = TskConfig {
            docker: Default::default(),
            git_town: Default::default(),
            project: project_configs,
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + named volume (prefixed with tsk-)
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("tsk-build-cache:/container/cache"))
        );
    }

    #[tokio::test]
    async fn test_project_volume_mounts_readonly() {
        use crate::context::{BindMount, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a readonly bind mount
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/etc/ssl/certs".to_string(),
                    container: "/etc/ssl/certs".to_string(),
                    readonly: true,
                })],
            },
        );
        let tsk_config = TskConfig {
            docker: Default::default(),
            git_town: Default::default(),
            project: project_configs,
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + readonly bind mount
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("/etc/ssl/certs:/etc/ssl/certs:ro"))
        );
    }

    #[tokio::test]
    async fn test_no_project_volumes_when_project_not_configured() {
        // Test that binds don't include project volumes when project is not in config
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.project = "unconfigured-project".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should only have base binds: workspace, claude dir, instructions, output
        assert_eq!(binds.len(), 4);
    }
}
