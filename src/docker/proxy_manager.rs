//! Proxy management for TSK
//!
//! This module provides centralized management for the TSK proxy infrastructure,
//! handling proxy container lifecycle, health checks, and network configuration.
//!
//! Proxy containers are fingerprinted by their configuration (host_ports and
//! squid_conf content). Tasks with identical proxy configurations share a proxy
//! container, while tasks with different configurations get separate instances.

use crate::context::ContainerEngine;
use crate::context::ResolvedProxyConfig;
use crate::context::docker_client::DockerClient;
use crate::context::tsk_env::TskEnv;
use anyhow::{Context, Result};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::RemoveContainerOptions;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Network name prefix for agent isolated networks
const TSK_AGENT_NETWORK_PREFIX: &str = "tsk-agent-";
/// Docker image name for the proxy
const PROXY_IMAGE: &str = "tsk/proxy";
/// Proxy port
const PROXY_PORT: &str = "3128/tcp";

/// Manages TSK proxy container lifecycle with per-configuration instances.
///
/// Each unique proxy configuration (host_ports + squid_conf) gets its own
/// proxy container identified by a fingerprint. Tasks with identical proxy
/// configurations share the same proxy container.
pub struct ProxyManager {
    docker_client: Arc<dyn DockerClient>,
    tsk_env: Arc<TskEnv>,
    container_engine: ContainerEngine,
}

impl ProxyManager {
    /// Creates a new ProxyManager with the given Docker client and environment.
    ///
    /// Proxy configuration is provided per-call via `ResolvedProxyConfig` rather
    /// than stored on the manager, enabling multiple proxy instances.
    pub fn new(
        client: Arc<dyn DockerClient>,
        tsk_env: Arc<TskEnv>,
        container_engine: ContainerEngine,
    ) -> Self {
        Self {
            docker_client: client,
            tsk_env,
            container_engine,
        }
    }

    /// Ensures the proxy for the given configuration is running and healthy.
    ///
    /// This method:
    /// 1. Checks if the proxy for this config is already running (skips build if so)
    /// 2. Builds the proxy image if needed
    /// 3. Ensures the external network exists
    /// 4. Starts the proxy container if not running
    /// 5. Waits for the proxy to become healthy
    ///
    /// # Returns
    /// * `Ok(container_name)` - the proxy container name on success
    /// * `Err` if proxy cannot be started or becomes unhealthy
    pub async fn ensure_proxy(
        &self,
        proxy_config: &ResolvedProxyConfig,
        build_log_path: Option<&std::path::Path>,
    ) -> Result<String> {
        let container_name = proxy_config.proxy_container_name();

        // Skip build if proxy is already running - config changes will be
        // picked up when the proxy is stopped and restarted
        if !self.is_proxy_running(&container_name).await? {
            self.build_proxy(false, build_log_path)
                .await
                .context("Failed to build proxy image")?;
        }

        // Ensure external network exists
        let network_name = proxy_config.external_network_name();
        self.ensure_network(&network_name)
            .await
            .context("Failed to ensure network exists")?;

        // Check if proxy container exists and is running
        self.ensure_proxy_container(proxy_config)
            .await
            .context("Failed to ensure proxy container is running")?;

        // Wait for proxy to be healthy
        self.wait_for_proxy_health(&container_name)
            .await
            .context("Failed to wait for proxy health")?;

        Ok(container_name)
    }

    /// Builds the generic proxy Docker image.
    ///
    /// The image is always built without custom squid.conf baked in. Per-configuration
    /// squid.conf is mounted at container start time via bind mount.
    ///
    /// # Arguments
    /// * `no_cache` - Whether to build without using Docker's cache
    /// * `build_log_path` - Optional path to save build output on failure
    pub async fn build_proxy(
        &self,
        no_cache: bool,
        build_log_path: Option<&std::path::Path>,
    ) -> Result<()> {
        println!("Building proxy image: {PROXY_IMAGE}");

        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::assets::utils::extract_dockerfile_to_temp;

        // Warn about deprecated legacy squid.conf file
        let legacy_squid_conf = self.tsk_env.config_dir().join("squid.conf");
        if legacy_squid_conf.exists() {
            eprintln!(
                "Warning: {} is deprecated. Use squid_conf or squid_conf_path in tsk.toml instead.",
                legacy_squid_conf.display()
            );
        }

        // Extract dockerfile to temporary directory
        let asset_manager = EmbeddedAssetManager;
        let dockerfile_dir = extract_dockerfile_to_temp(&asset_manager, "tsk-proxy")
            .context("Failed to extract proxy Dockerfile")?;

        // Create tar archive from the proxy dockerfile directory
        let tar_archive = self
            .create_tar_archive_from_directory(&dockerfile_dir)
            .context("Failed to create tar archive for proxy build")?;

        // Clean up the temporary directory
        let _ = std::fs::remove_dir_all(&dockerfile_dir);

        // Build options for proxy
        let mut options_builder = bollard::query_parameters::BuildImageOptionsBuilder::default();
        options_builder = options_builder.dockerfile("Dockerfile");
        options_builder = options_builder.t(PROXY_IMAGE);
        options_builder = options_builder.nocache(no_cache);
        if self.container_engine == ContainerEngine::Podman {
            options_builder = options_builder.networkmode("host");
        }
        let options = options_builder.build();

        // Build the image using the DockerClient
        let mut build_stream = self
            .docker_client
            .build_image(options, tar_archive)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build proxy image: {e}"))?;

        // Capture build output (only displayed on failure to reduce noise)
        use futures_util::StreamExt;
        let mut build_output = String::new();
        while let Some(result) = build_stream.next().await {
            match result {
                Ok(line) => {
                    build_output.push_str(&line);
                }
                Err(e) => {
                    if let Some(log_path) = build_log_path {
                        super::save_build_log(log_path, &build_output);
                    }
                    eprint!("{build_output}");
                    return Err(anyhow::anyhow!("Failed to build proxy image: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Stops and removes the specified proxy container.
    pub async fn stop_proxy(&self, container_name: &str) -> Result<()> {
        println!("Stopping TSK proxy container {container_name}...");

        match self
            .docker_client
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => {
                println!("Proxy container {container_name} stopped successfully");
                Ok(())
            }
            Err(e) if e.to_lowercase().contains("no such container") => {
                println!("Proxy container {container_name} was not running");
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!(
                "Failed to stop proxy container {container_name}: {e}"
            )),
        }
    }

    /// Checks if the specified proxy container is currently running.
    pub async fn is_proxy_running(&self, container_name: &str) -> Result<bool> {
        match self.docker_client.inspect_container(container_name).await {
            Ok(json_data) => {
                let data: serde_json::Value = serde_json::from_str(&json_data)
                    .map_err(|e| anyhow::anyhow!("Failed to parse container info: {e}"))?;
                Ok(data
                    .get("State")
                    .and_then(|s| s.get("Running"))
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false))
            }
            Err(e) if e.to_lowercase().contains("no such container") => Ok(false),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Counts the number of agent networks the specified proxy is connected to.
    ///
    /// Inspects the proxy container and counts networks matching the
    /// `tsk-agent-*` pattern (excluding the external network).
    pub async fn count_connected_agents(&self, container_name: &str) -> Result<usize> {
        // Inspect the proxy container to get its network connections
        match self.docker_client.inspect_container(container_name).await {
            Ok(json_data) => {
                let data: serde_json::Value = serde_json::from_str(&json_data)
                    .map_err(|e| anyhow::anyhow!("Failed to parse container info: {e}"))?;

                // Count networks, excluding the external network
                let count = data
                    .get("NetworkSettings")
                    .and_then(|ns| ns.get("Networks"))
                    .and_then(|n| n.as_object())
                    .map(|networks| {
                        networks
                            .keys()
                            .filter(|name| name.starts_with(TSK_AGENT_NETWORK_PREFIX))
                            .count()
                    })
                    .unwrap_or(0);

                Ok(count)
            }
            Err(e) if e.to_lowercase().contains("no such container") => Ok(0),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Conditionally stops the proxy for the given config if no agents are connected.
    ///
    /// # Returns
    /// * `Ok(true)` if proxy was stopped
    /// * `Ok(false)` if proxy is still in use or was not running
    pub async fn maybe_stop_proxy(&self, proxy_config: &ResolvedProxyConfig) -> Result<bool> {
        let container_name = proxy_config.proxy_container_name();

        // Check if proxy is even running
        if !self.is_proxy_running(&container_name).await? {
            return Ok(false);
        }

        // Count connected agents
        let agent_count = self.count_connected_agents(&container_name).await?;

        if agent_count == 0 {
            self.stop_proxy(&container_name).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Ensures the specified external network exists for proxy internet access.
    async fn ensure_network(&self, network_name: &str) -> Result<()> {
        if !self
            .docker_client
            .network_exists(network_name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check if network exists: {e}"))?
        {
            self.docker_client
                .create_network(network_name)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create network: {e}"))?;
        }
        Ok(())
    }

    /// Ensures the proxy container for the given config is running.
    ///
    /// Uses the fingerprinted container and network names from the proxy config.
    /// When `proxy_config.squid_conf` is set, writes the content to a host file
    /// and bind-mounts it into the container.
    async fn ensure_proxy_container(&self, proxy_config: &ResolvedProxyConfig) -> Result<()> {
        let container_name = proxy_config.proxy_container_name();
        let network_name = proxy_config.external_network_name();
        let host_ports_env = format!("TSK_HOST_PORTS={}", proxy_config.host_ports_env());

        // Prepare optional squid.conf bind mount
        let binds = if let Some(ref squid_conf_content) = proxy_config.squid_conf {
            let fingerprint = proxy_config.fingerprint();
            let proxy_conf_dir = self.tsk_env.proxy_config_dir(&fingerprint);
            std::fs::create_dir_all(&proxy_conf_dir)
                .context("Failed to create proxy config directory")?;

            let squid_conf_path = proxy_conf_dir.join("squid.conf");
            std::fs::write(&squid_conf_path, squid_conf_content)
                .context("Failed to write squid.conf")?;

            Some(vec![format!(
                "{}:/etc/squid/squid.conf:ro",
                squid_conf_path.display()
            )])
        } else {
            None
        };

        let container_config = ContainerCreateBody {
            image: Some(PROXY_IMAGE.to_string()),
            exposed_ports: Some(vec![PROXY_PORT.to_string()]),
            env: Some(vec![host_ports_env]),
            host_config: Some(HostConfig {
                network_mode: Some(network_name),
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
                restart_policy: Some(bollard::models::RestartPolicy {
                    name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                binds,
                // Security hardening options
                readonly_rootfs: Some(true),
                cap_drop: Some(vec!["ALL".to_string()]),
                cap_add: Some(vec![
                    "NET_ADMIN".to_string(), // For iptables firewall rules
                    "SETUID".to_string(),    // For su-exec to drop privileges
                    "SETGID".to_string(),    // For su-exec to drop privileges
                    "CHOWN".to_string(),     // For fixing tmpfs ownership at startup
                ]),
                security_opt: Some(vec!["no-new-privileges:true".to_string()]),
                tmpfs: Some(HashMap::from([
                    ("/var/cache/squid".to_string(), "size=10m".to_string()),
                    ("/var/log/squid".to_string(), "size=50m".to_string()),
                    ("/var/run/squid".to_string(), "size=1m".to_string()),
                ])),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        // Try to create the container (this will fail if it already exists)
        match self
            .docker_client
            .create_container(Some(create_options), container_config)
            .await
        {
            Ok(_) => {
                // New container created, start it
                self.docker_client
                    .start_container(&container_name)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to start proxy container: {e}"))?;
            }
            Err(e) => {
                // Container might already exist, try to start it
                if e.to_lowercase().contains("already in use") {
                    match self.docker_client.start_container(&container_name).await {
                        Ok(_) => (),
                        Err(e) if e.to_lowercase().contains("already started") => (),
                        Err(e) => {
                            return Err(anyhow::anyhow!("Failed to start proxy container: {e}"));
                        }
                    }
                } else {
                    return Err(anyhow::anyhow!("Failed to create proxy container: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Waits for the specified proxy container to become healthy.
    async fn wait_for_proxy_health(&self, container_name: &str) -> Result<()> {
        const MAX_RETRIES: u32 = 30; // 30 retries with 1 second delay = 30 seconds max wait
        const RETRY_DELAY_MS: u64 = 1000;

        for attempt in 1..=MAX_RETRIES {
            match self.docker_client.inspect_container(container_name).await {
                Ok(json_data) => {
                    // Parse the JSON to check health status
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&json_data) {
                        // Check if container has a health check
                        if let Some(state) = data.get("State") {
                            // Check if container is running
                            if let Some(running) = state.get("Running").and_then(|v| v.as_bool())
                                && !running
                            {
                                return Err(anyhow::anyhow!("Proxy container is not running"));
                            }

                            // Check health status if it exists
                            if let Some(health) = state.get("Health") {
                                if let Some(status) = health.get("Status").and_then(|v| v.as_str())
                                {
                                    match status {
                                        "healthy" => {
                                            println!("Proxy container is healthy");
                                            return Ok(());
                                        }
                                        "unhealthy" => {
                                            return Err(anyhow::anyhow!(
                                                "Proxy container is unhealthy"
                                            ));
                                        }
                                        "starting" => {
                                            // Still starting, continue waiting
                                            if attempt == 1 {
                                                println!(
                                                    "Waiting for proxy container to become healthy..."
                                                );
                                            }
                                        }
                                        _ => {
                                            // Unknown status, continue waiting
                                        }
                                    }
                                }
                            } else {
                                // No health check configured, just verify it's running
                                // This is for backward compatibility
                                println!("Proxy container is running (no health check configured)");
                                return Ok(());
                            }
                        }
                    }
                }
                Err(e) if e.to_lowercase().contains("no such container") => {
                    return Err(anyhow::anyhow!("Proxy container not found"));
                }
                Err(_) => {
                    // Ignore other errors and retry
                }
            }

            if attempt < MAX_RETRIES {
                tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }

        Err(anyhow::anyhow!(
            "Proxy container failed to become healthy after {} seconds",
            MAX_RETRIES
        ))
    }

    /// Creates a tar archive from a directory
    fn create_tar_archive_from_directory(&self, dir_path: &Path) -> Result<Vec<u8>> {
        use tar::Builder;

        let mut tar_data = Vec::new();
        {
            let mut builder = Builder::new(&mut tar_data);

            // Add all files from the directory to the tar archive
            builder.append_dir_all(".", dir_path)?;

            builder.finish()?;
        }

        Ok(tar_data)
    }

    /// Creates an internal network for a specific agent container.
    ///
    /// The network is created with the `internal` flag, meaning it has no
    /// external route to the internet. The agent can only reach the proxy.
    ///
    /// # Returns
    /// The network name on success
    pub async fn create_agent_network(&self, task_id: &str) -> Result<String> {
        let network_name = Self::agent_network_name(task_id);

        self.docker_client
            .create_internal_network(&network_name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create agent network: {e}"))?;

        Ok(network_name)
    }

    /// Connects the specified proxy container to an agent's isolated network.
    ///
    /// This must be called BEFORE starting the agent container so the agent
    /// can reach the proxy.
    pub async fn connect_proxy_to_network(
        &self,
        proxy_container_name: &str,
        network_name: &str,
    ) -> Result<()> {
        self.docker_client
            .connect_container_to_network(proxy_container_name, network_name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect proxy to network: {e}"))
    }

    /// Cleans up an agent's network after task completion.
    ///
    /// Disconnects the proxy from the network and removes it.
    /// Logs warnings on failure but does not return errors (cleanup is best-effort).
    pub async fn cleanup_agent_network(&self, proxy_container_name: &str, network_name: &str) {
        // Disconnect proxy from network
        if let Err(e) = self
            .docker_client
            .disconnect_container_from_network(proxy_container_name, network_name)
            .await
        {
            eprintln!("Warning: Failed to disconnect proxy from network {network_name}: {e}");
        }

        // Remove the network
        if let Err(e) = self.docker_client.remove_network(network_name).await {
            eprintln!("Warning: Failed to remove network {network_name}: {e}");
        }
    }

    /// Gets the network name for a specific agent task
    ///
    /// # Arguments
    /// * `task_id` - The task identifier
    pub fn agent_network_name(task_id: &str) -> String {
        format!("{TSK_AGENT_NETWORK_PREFIX}{task_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::test_utils::TrackedDockerClient;

    fn default_proxy_config() -> ResolvedProxyConfig {
        ResolvedProxyConfig {
            host_ports: vec![],
            squid_conf: None,
        }
    }

    #[tokio::test]
    async fn test_ensure_proxy_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let result = manager.ensure_proxy(&proxy_config, None).await;

        assert!(result.is_ok());
        let container_name = result.unwrap();
        assert_eq!(container_name, proxy_config.proxy_container_name());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 1);

        let (options, config) = &create_calls[0];
        assert_eq!(
            options.as_ref().unwrap().name,
            Some(proxy_config.proxy_container_name())
        );

        // Verify extra_hosts is set for host.docker.internal access
        let host_config = config.host_config.as_ref().unwrap();
        let extra_hosts = host_config.extra_hosts.as_ref().unwrap();
        assert!(extra_hosts.contains(&"host.docker.internal:host-gateway".to_string()));

        // Verify env includes TSK_HOST_PORTS (empty by default)
        let env = config.env.as_ref().unwrap();
        assert!(env.iter().any(|e| e.starts_with("TSK_HOST_PORTS=")));

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 1);
        assert_eq!(start_calls[0], proxy_config.proxy_container_name());
    }

    #[tokio::test]
    async fn test_ensure_proxy_with_host_ports() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = ResolvedProxyConfig {
            host_ports: vec![5432, 6379],
            squid_conf: None,
        };
        let result = manager.ensure_proxy(&proxy_config, None).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let (_, config) = &create_calls[0];

        // Verify env includes configured host ports (sorted)
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"TSK_HOST_PORTS=5432,6379".to_string()));
    }

    #[tokio::test]
    async fn test_stop_proxy_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let container_name = proxy_config.proxy_container_name();
        let result = manager.stop_proxy(&container_name).await;

        assert!(result.is_ok());

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, container_name);
        assert!(remove_calls[0].1.as_ref().unwrap().force);
    }

    #[tokio::test]
    async fn test_stop_proxy_container_not_found() {
        use crate::context::docker_client::DockerClient;
        use async_trait::async_trait;
        use bollard::models::ContainerCreateBody;
        use bollard::query_parameters::*;
        use futures_util::Stream;

        struct NoContainerDockerClient;

        #[async_trait]
        impl DockerClient for NoContainerDockerClient {
            async fn remove_container(
                &self,
                _id: &str,
                _options: Option<RemoveContainerOptions>,
            ) -> Result<(), String> {
                Err("No such container: tsk-proxy-abcd1234".to_string())
            }

            async fn create_container(
                &self,
                _options: Option<CreateContainerOptions>,
                _config: ContainerCreateBody,
            ) -> Result<String, String> {
                Ok("test-id".to_string())
            }

            async fn start_container(&self, _id: &str) -> Result<(), String> {
                Ok(())
            }

            async fn wait_container(&self, _id: &str) -> Result<i64, String> {
                Ok(0)
            }

            async fn kill_container(&self, _id: &str) -> Result<(), String> {
                Ok(())
            }

            async fn logs(
                &self,
                _id: &str,
                _options: Option<LogsOptions>,
            ) -> Result<String, String> {
                Ok("".to_string())
            }

            async fn logs_stream(
                &self,
                _id: &str,
                _options: Option<LogsOptions>,
            ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String>
            {
                use futures_util::stream;
                let stream = stream::once(async { Ok("".to_string()) });
                Ok(Box::new(Box::pin(stream)))
            }

            async fn create_network(&self, _name: &str) -> Result<String, String> {
                Ok("network-id".to_string())
            }

            async fn network_exists(&self, _name: &str) -> Result<bool, String> {
                Ok(true)
            }

            async fn build_image(
                &self,
                _options: BuildImageOptions,
                _tar_archive: Vec<u8>,
            ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String>
            {
                use futures_util::stream;
                let stream = stream::once(async { Ok("Building...".to_string()) });
                Ok(Box::new(Box::pin(stream)))
            }

            async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
                Ok(true)
            }

            async fn inspect_container(&self, _id: &str) -> Result<String, String> {
                Ok(r#"{"State": {"Health": {"Status": "healthy"}}}"#.to_string())
            }

            async fn attach_container(&self, _id: &str) -> Result<(), String> {
                Ok(())
            }

            async fn upload_to_container(
                &self,
                _id: &str,
                _dest_path: &str,
                _tar_data: Vec<u8>,
            ) -> Result<(), String> {
                Ok(())
            }

            async fn create_internal_network(&self, _name: &str) -> Result<String, String> {
                Ok("internal-network-id".to_string())
            }

            async fn connect_container_to_network(
                &self,
                _container: &str,
                _network: &str,
            ) -> Result<(), String> {
                Ok(())
            }

            async fn disconnect_container_from_network(
                &self,
                _container: &str,
                _network: &str,
            ) -> Result<(), String> {
                Ok(())
            }

            async fn remove_network(&self, _name: &str) -> Result<(), String> {
                Ok(())
            }

            async fn ping(&self) -> Result<String, String> {
                Ok("OK".to_string())
            }
        }

        let mock_client: Arc<dyn DockerClient> = Arc::new(NoContainerDockerClient);
        let ctx = AppContext::builder().build();

        let manager = ProxyManager::new(mock_client, ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let result = manager
            .stop_proxy(&proxy_config.proxy_container_name())
            .await;

        // Should succeed even if container doesn't exist
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_for_proxy_health_success() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true,
                    "Health": {
                        "Status": "healthy"
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.wait_for_proxy_health("tsk-proxy-test").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_for_proxy_health_unhealthy() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true,
                    "Health": {
                        "Status": "unhealthy"
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.wait_for_proxy_health("tsk-proxy-test").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unhealthy"));
    }

    #[tokio::test]
    async fn test_wait_for_proxy_health_no_health_check() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.wait_for_proxy_health("tsk-proxy-test").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_for_proxy_health_not_running() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": false
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.wait_for_proxy_health("tsk-proxy-test").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[test]
    fn test_agent_network_name() {
        assert_eq!(
            ProxyManager::agent_network_name("test-task-123"),
            "tsk-agent-test-task-123"
        );
        assert_eq!(
            ProxyManager::agent_network_name("2024-01-15-feat-auth"),
            "tsk-agent-2024-01-15-feat-auth"
        );
    }

    #[tokio::test]
    async fn test_create_agent_network() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.create_agent_network("test-task-123").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tsk-agent-test-task-123");

        let calls = mock_client.create_internal_network_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "tsk-agent-test-task-123");
    }

    #[tokio::test]
    async fn test_create_agent_network_error() {
        let mock_client = Arc::new(TrackedDockerClient {
            create_internal_network_error: Some("Network creation failed".to_string()),
            ..Default::default()
        });
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.create_agent_network("test-task-123").await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to create agent network")
        );
    }

    #[tokio::test]
    async fn test_connect_proxy_to_network() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let container_name = proxy_config.proxy_container_name();
        let result = manager
            .connect_proxy_to_network(&container_name, "tsk-agent-test-123")
            .await;

        assert!(result.is_ok());

        let calls = mock_client.connect_network_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (container_name, "tsk-agent-test-123".to_string()));
    }

    #[tokio::test]
    async fn test_cleanup_agent_network() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let container_name = proxy_config.proxy_container_name();
        manager
            .cleanup_agent_network(&container_name, "tsk-agent-test-123")
            .await;

        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        assert_eq!(
            disconnect_calls[0],
            (container_name, "tsk-agent-test-123".to_string())
        );

        let remove_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0], "tsk-agent-test-123");
    }

    #[tokio::test]
    async fn test_cleanup_agent_network_handles_errors_gracefully() {
        let mock_client = Arc::new(TrackedDockerClient {
            remove_network_error: Some("Network in use".to_string()),
            ..Default::default()
        });
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        manager
            .cleanup_agent_network(&proxy_config.proxy_container_name(), "tsk-agent-test-123")
            .await;

        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);

        let remove_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_build_proxy_without_custom_squid_conf() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);

        let result = manager.build_proxy(false, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_is_proxy_running_true() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager = ProxyManager::new(mock_client, ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.is_proxy_running("tsk-proxy-test").await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_is_proxy_running_false() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": false
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager = ProxyManager::new(mock_client, ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.is_proxy_running("tsk-proxy-test").await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_count_connected_agents() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "NetworkSettings": {
                    "Networks": {
                        "tsk-external": {},
                        "tsk-agent-task1": {},
                        "tsk-agent-task2": {},
                        "tsk-agent-task3": {}
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager = ProxyManager::new(mock_client, ctx.tsk_env(), ContainerEngine::Docker);
        let result = manager.count_connected_agents("tsk-proxy-test").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_maybe_stop_proxy_no_agents() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                },
                "NetworkSettings": {
                    "Networks": {
                        "tsk-external": {}
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let result = manager.maybe_stop_proxy(&proxy_config).await;

        assert!(result.is_ok());
        assert!(result.unwrap()); // Should have stopped

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, proxy_config.proxy_container_name());
    }

    #[tokio::test]
    async fn test_maybe_stop_proxy_with_agents() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                },
                "NetworkSettings": {
                    "Networks": {
                        "tsk-external": {},
                        "tsk-agent-task1": {},
                        "tsk-agent-task2": {}
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = default_proxy_config();
        let result = manager.maybe_stop_proxy(&proxy_config).await;

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should NOT have stopped

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_ensure_proxy_with_squid_conf_mount() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();

        let manager =
            ProxyManager::new(mock_client.clone(), ctx.tsk_env(), ContainerEngine::Docker);
        let proxy_config = ResolvedProxyConfig {
            host_ports: vec![],
            squid_conf: Some("http_port 3128\nacl custom src all".to_string()),
        };
        let result = manager.ensure_proxy(&proxy_config, None).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let (_, config) = &create_calls[0];

        // Verify bind mount includes squid.conf
        let host_config = config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 1);
        assert!(binds[0].ends_with("squid.conf:/etc/squid/squid.conf:ro"));

        // Verify the squid.conf file was written to disk
        let fingerprint = proxy_config.fingerprint();
        let squid_path = ctx
            .tsk_env()
            .proxy_config_dir(&fingerprint)
            .join("squid.conf");
        assert!(squid_path.exists());
        let content = std::fs::read_to_string(&squid_path).unwrap();
        assert_eq!(content, "http_port 3128\nacl custom src all");
    }
}
