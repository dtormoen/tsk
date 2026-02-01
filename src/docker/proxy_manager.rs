//! Proxy management for TSK
//!
//! This module provides centralized management for the TSK proxy infrastructure,
//! handling proxy container lifecycle, health checks, and network configuration.

use crate::context::AppContext;
use crate::context::docker_client::DockerClient;
use crate::context::tsk_env::TskEnv;
use anyhow::{Context, Result};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::RemoveContainerOptions;
use std::path::Path;
use std::sync::Arc;

/// Network name for TSK containers
const TSK_NETWORK_NAME: &str = "tsk-network";
/// Container name for the proxy
const PROXY_CONTAINER_NAME: &str = "tsk-proxy";
/// Docker image name for the proxy
const PROXY_IMAGE: &str = "tsk/proxy";
/// Proxy port
const PROXY_PORT: &str = "3128/tcp";

/// Manages the TSK proxy container lifecycle
///
/// This struct provides a high-level interface for managing the proxy container
/// that provides controlled network access for TSK task containers. It handles
/// proxy image building, container creation, health monitoring, and cleanup.
///
/// The proxy manager supports custom Squid configuration by checking for a
/// `squid.conf` file in the TSK config directory. If found, it will use that
/// configuration instead of the default embedded one.
pub struct ProxyManager {
    docker_client: Arc<dyn DockerClient>,
    tsk_env: Arc<TskEnv>,
}

impl ProxyManager {
    /// Creates a new ProxyManager from AppContext
    ///
    /// # Arguments
    /// * `ctx` - Application context with all dependencies
    pub fn new(ctx: &AppContext) -> Self {
        Self {
            docker_client: ctx.docker_client(),
            tsk_env: ctx.tsk_env(),
        }
    }

    /// Ensures the proxy is running and healthy
    ///
    /// This method:
    /// 1. Checks if proxy is already running (skips build if so)
    /// 2. Builds the proxy image if needed
    /// 3. Ensures the network exists
    /// 4. Starts the proxy container if not running
    /// 5. Waits for the proxy to become healthy
    ///
    /// Config changes are picked up when the proxy is stopped (manually or
    /// automatically when no agents are connected) and then restarted.
    ///
    /// # Returns
    /// * `Ok(())` if proxy is running and healthy
    /// * `Err` if proxy cannot be started or becomes unhealthy
    pub async fn ensure_proxy(&self) -> Result<()> {
        // Skip build if proxy is already running - config changes will be
        // picked up when the proxy is stopped and restarted
        if !self.is_proxy_running().await? {
            self.build_proxy(false)
                .await
                .context("Failed to build proxy image")?;
        }

        // Ensure network exists
        self.ensure_network()
            .await
            .context("Failed to ensure network exists")?;

        // Check if proxy container exists and is running
        self.ensure_proxy_container()
            .await
            .context("Failed to ensure proxy container is running")?;

        // Wait for proxy to be healthy
        self.wait_for_proxy_health()
            .await
            .context("Failed to wait for proxy health")?;

        Ok(())
    }

    /// Builds the proxy Docker image
    ///
    /// This method will check for a custom squid.conf file in the TSK config directory.
    /// If found, it will use that configuration instead of the default embedded one.
    ///
    /// # Arguments
    /// * `no_cache` - Whether to build without using Docker's cache
    ///
    /// # Returns
    /// * `Ok(())` if build succeeds
    /// * `Err` if build fails
    pub async fn build_proxy(&self, no_cache: bool) -> Result<()> {
        println!("Building proxy image: {PROXY_IMAGE}");

        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::assets::utils::extract_dockerfile_to_temp;

        // Extract dockerfile to temporary directory
        let asset_manager = EmbeddedAssetManager;
        let dockerfile_dir = extract_dockerfile_to_temp(&asset_manager, "tsk-proxy")
            .context("Failed to extract proxy Dockerfile")?;

        // Check for custom squid.conf in config directory
        let custom_squid_conf_path = self.tsk_env.config_dir().join("squid.conf");
        if custom_squid_conf_path.exists() {
            println!(
                "Using custom squid.conf from {}",
                custom_squid_conf_path.display()
            );
            // Copy the custom squid.conf to the build directory, replacing the default one
            let dest_squid_conf = dockerfile_dir.join("squid.conf");
            std::fs::copy(&custom_squid_conf_path, &dest_squid_conf)
                .context("Failed to copy custom squid.conf")?;
        }

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
        let options = options_builder.build();

        // Build the image using the DockerClient with streaming output
        let mut build_stream = self
            .docker_client
            .build_image(options, tar_archive)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build proxy image: {e}"))?;

        // Stream build output for real-time visibility
        use futures_util::StreamExt;
        while let Some(result) = build_stream.next().await {
            match result {
                Ok(line) => {
                    print!("{line}");
                    // Ensure output is flushed immediately
                    use std::io::Write;
                    std::io::stdout().flush().unwrap_or(());
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to build proxy image: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Stops and removes the proxy container
    ///
    /// # Returns
    /// * `Ok(())` if proxy is stopped or was not running
    /// * `Err` if proxy cannot be stopped
    pub async fn stop_proxy(&self) -> Result<()> {
        println!("Stopping TSK proxy container...");

        match self
            .docker_client
            .remove_container(
                PROXY_CONTAINER_NAME,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => {
                println!("Proxy container stopped successfully");
                Ok(())
            }
            Err(e) if e.contains("No such container") => {
                println!("Proxy container was not running");
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to stop proxy container: {e}")),
        }
    }

    /// Checks if the proxy container is currently running
    ///
    /// # Returns
    /// * `Ok(true)` if proxy container is running
    /// * `Ok(false)` if proxy container is not running or doesn't exist
    /// * `Err` if unable to inspect the container
    pub async fn is_proxy_running(&self) -> Result<bool> {
        match self
            .docker_client
            .inspect_container(PROXY_CONTAINER_NAME)
            .await
        {
            Ok(json_data) => {
                let data: serde_json::Value = serde_json::from_str(&json_data)
                    .map_err(|e| anyhow::anyhow!("Failed to parse container info: {e}"))?;
                Ok(data
                    .get("State")
                    .and_then(|s| s.get("Running"))
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false))
            }
            Err(e) if e.contains("No such container") => Ok(false),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Counts the number of agent containers connected to the TSK network
    ///
    /// This excludes the proxy container itself from the count.
    ///
    /// # Returns
    /// * `Ok(count)` - Number of agent containers connected
    /// * `Err` if unable to inspect the network
    pub async fn count_connected_agents(&self) -> Result<usize> {
        self.docker_client
            .count_network_containers(TSK_NETWORK_NAME, PROXY_CONTAINER_NAME)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// Conditionally stops the proxy if no agents are connected
    ///
    /// This method checks if any agent containers are using the proxy network.
    /// If no agents are connected, it stops the proxy to ensure a fresh rebuild
    /// on next use.
    ///
    /// # Returns
    /// * `Ok(true)` if proxy was stopped
    /// * `Ok(false)` if proxy is still in use or was not running
    /// * `Err` if unable to check status or stop proxy
    pub async fn maybe_stop_proxy(&self) -> Result<bool> {
        // Check if proxy is even running
        if !self.is_proxy_running().await? {
            return Ok(false);
        }

        // Count connected agents
        let agent_count = self.count_connected_agents().await?;

        if agent_count == 0 {
            self.stop_proxy().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Ensures the TSK network exists
    async fn ensure_network(&self) -> Result<()> {
        if !self
            .docker_client
            .network_exists(TSK_NETWORK_NAME)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check if network exists: {e}"))?
        {
            self.docker_client
                .create_network(TSK_NETWORK_NAME)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create network: {e}"))?;
        }
        Ok(())
    }

    /// Ensures the proxy container is running
    async fn ensure_proxy_container(&self) -> Result<()> {
        // Create proxy container configuration
        let proxy_config = ContainerCreateBody {
            image: Some(PROXY_IMAGE.to_string()),
            exposed_ports: Some(vec![PROXY_PORT.to_string()]),
            host_config: Some(HostConfig {
                network_mode: Some(TSK_NETWORK_NAME.to_string()),
                restart_policy: Some(bollard::models::RestartPolicy {
                    name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(PROXY_CONTAINER_NAME)
            .build();

        // Try to create the container (this will fail if it already exists)
        match self
            .docker_client
            .create_container(Some(create_options), proxy_config)
            .await
        {
            Ok(_) => {
                // New container created, start it
                self.docker_client
                    .start_container(PROXY_CONTAINER_NAME)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to start proxy container: {e}"))?;
            }
            Err(e) => {
                // Container might already exist, try to start it
                if e.contains("already in use") {
                    // Try to start existing container
                    match self
                        .docker_client
                        .start_container(PROXY_CONTAINER_NAME)
                        .await
                    {
                        Ok(_) => (),
                        Err(e) if e.contains("already started") => (),
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

    /// Waits for the proxy container to become healthy
    async fn wait_for_proxy_health(&self) -> Result<()> {
        const MAX_RETRIES: u32 = 30; // 30 retries with 1 second delay = 30 seconds max wait
        const RETRY_DELAY_MS: u64 = 1000;

        for attempt in 1..=MAX_RETRIES {
            match self
                .docker_client
                .inspect_container(PROXY_CONTAINER_NAME)
                .await
            {
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
                Err(e) if e.contains("No such container") => {
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

    /// Gets the network name used by the proxy
    pub fn network_name(&self) -> &str {
        TSK_NETWORK_NAME
    }

    /// Gets the proxy URL for environment configuration
    pub fn proxy_url(&self) -> String {
        format!(
            "http://{}:{}",
            PROXY_CONTAINER_NAME,
            PROXY_PORT.trim_end_matches("/tcp")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::test_utils::TrackedDockerClient;

    #[tokio::test]
    async fn test_ensure_proxy_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.ensure_proxy().await;

        assert!(result.is_ok());

        // Verify the expected calls were made
        // The mock client's network_exists field determines if the network exists
        // We can verify create_container was called for the proxy
        assert!(mock_client.network_exists);

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(
            create_calls[0].0.as_ref().unwrap().name,
            Some(PROXY_CONTAINER_NAME.to_string())
        );

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 1);
        assert_eq!(start_calls[0], PROXY_CONTAINER_NAME);
    }

    #[tokio::test]
    async fn test_stop_proxy_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.stop_proxy().await;

        assert!(result.is_ok());

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, PROXY_CONTAINER_NAME);
        assert!(remove_calls[0].1.as_ref().unwrap().force);
    }

    #[tokio::test]
    async fn test_stop_proxy_container_not_found() {
        // Test that stop_proxy succeeds even when container doesn't exist
        // We'll create a custom DockerClient implementation for this test
        use crate::context::docker_client::DockerClient;
        use async_trait::async_trait;
        use bollard::models::ContainerCreateBody;
        use bollard::query_parameters::*;
        use futures_util::Stream;

        struct NoContainerDockerClient;

        #[async_trait]
        impl DockerClient for NoContainerDockerClient {
            #[cfg(test)]
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }

            async fn remove_container(
                &self,
                _id: &str,
                _options: Option<RemoveContainerOptions>,
            ) -> Result<(), String> {
                Err("No such container: tsk-proxy".to_string())
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

            async fn count_network_containers(
                &self,
                _network_name: &str,
                _exclude_container: &str,
            ) -> Result<usize, String> {
                Ok(0)
            }
        }

        let mock_client = Arc::new(NoContainerDockerClient);
        let ctx = AppContext::builder()
            .with_docker_client(mock_client)
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.stop_proxy().await;

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

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.wait_for_proxy_health().await;

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

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.wait_for_proxy_health().await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unhealthy"));
    }

    #[tokio::test]
    async fn test_wait_for_proxy_health_no_health_check() {
        use serde_json::json;

        // Test backward compatibility when no health check is configured
        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                }
            })
            .to_string(),
            ..Default::default()
        });

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.wait_for_proxy_health().await;

        // Should succeed for backward compatibility
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

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.wait_for_proxy_health().await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[test]
    fn test_proxy_url() {
        let ctx = AppContext::builder().build();
        let manager = ProxyManager::new(&ctx);

        assert_eq!(manager.proxy_url(), "http://tsk-proxy:3128");
        assert_eq!(manager.network_name(), "tsk-network");
    }

    #[tokio::test]
    async fn test_build_proxy_with_custom_squid_conf() {
        // Create a mock docker client that captures the build tar archive
        use crate::context::docker_client::DockerClient;
        use async_trait::async_trait;
        use bollard::models::ContainerCreateBody;
        use bollard::query_parameters::*;
        use futures_util::Stream;
        use std::sync::Mutex;

        struct CaptureDockerClient {
            tar_archive: Mutex<Option<Vec<u8>>>,
        }

        #[async_trait]
        impl DockerClient for CaptureDockerClient {
            #[cfg(test)]
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }

            async fn build_image(
                &self,
                _options: BuildImageOptions,
                tar_archive: Vec<u8>,
            ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String>
            {
                *self.tar_archive.lock().unwrap() = Some(tar_archive);
                use futures_util::stream;
                let stream = stream::once(async { Ok("Building...".to_string()) });
                Ok(Box::new(Box::pin(stream)))
            }

            async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
                Ok(false) // Force rebuild
            }

            async fn remove_container(
                &self,
                _id: &str,
                _options: Option<RemoveContainerOptions>,
            ) -> Result<(), String> {
                Ok(())
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

            async fn count_network_containers(
                &self,
                _network_name: &str,
                _exclude_container: &str,
            ) -> Result<usize, String> {
                Ok(0)
            }
        }

        let docker_client = Arc::new(CaptureDockerClient {
            tar_archive: Mutex::new(None),
        });

        // Create AppContext with test-safe temporary directories and custom docker client
        let ctx = AppContext::builder()
            .with_docker_client(docker_client.clone())
            .build();

        // Create a custom squid.conf file in the config directory
        let custom_squid_conf = ctx.tsk_env().config_dir().join("squid.conf");
        std::fs::write(
            &custom_squid_conf,
            "# Custom squid configuration\nhttp_port 3128\n",
        )
        .unwrap();

        let manager = ProxyManager::new(&ctx);
        let result = manager.build_proxy(false).await;

        assert!(result.is_ok());

        // Verify that the tar archive was created and contains the custom squid.conf
        let tar_data = docker_client.tar_archive.lock().unwrap();
        assert!(tar_data.is_some());

        // Extract and verify the tar archive contains our custom squid.conf
        use tar::Archive;
        let tar_bytes = tar_data.as_ref().unwrap();
        let mut archive = Archive::new(&tar_bytes[..]);

        let mut found_custom_squid = false;
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap();
            if path.to_str().unwrap().ends_with("squid.conf") {
                let mut content = String::new();
                use std::io::Read;
                entry.read_to_string(&mut content).unwrap();
                if content.contains("# Custom squid configuration") {
                    found_custom_squid = true;
                    break;
                }
            }
        }

        assert!(
            found_custom_squid,
            "Custom squid.conf should be in the tar archive"
        );
    }

    #[tokio::test]
    async fn test_build_proxy_without_custom_squid_conf() {
        // Test that default squid.conf is used when no custom one exists
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);

        // Just verify build_proxy doesn't error with default configuration
        let result = manager.build_proxy(false).await;
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

        let ctx = AppContext::builder()
            .with_docker_client(mock_client)
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.is_proxy_running().await;

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

        let ctx = AppContext::builder()
            .with_docker_client(mock_client)
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.is_proxy_running().await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_count_connected_agents() {
        let mock_client = Arc::new(TrackedDockerClient {
            network_container_count: 3,
            ..Default::default()
        });

        let ctx = AppContext::builder()
            .with_docker_client(mock_client)
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.count_connected_agents().await;

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
                }
            })
            .to_string(),
            network_container_count: 0,
            ..Default::default()
        });

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.maybe_stop_proxy().await;

        assert!(result.is_ok());
        assert!(result.unwrap()); // Should have stopped

        // Verify remove_container was called
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "tsk-proxy");
    }

    #[tokio::test]
    async fn test_maybe_stop_proxy_with_agents() {
        use serde_json::json;

        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: json!({
                "State": {
                    "Running": true
                }
            })
            .to_string(),
            network_container_count: 2, // Other agents connected
            ..Default::default()
        });

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();

        let manager = ProxyManager::new(&ctx);
        let result = manager.maybe_stop_proxy().await;

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should NOT have stopped

        // Verify remove_container was NOT called
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 0);
    }
}
