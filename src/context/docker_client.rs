use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use bollard::image::{BuildImageOptions, ListImagesOptions};
use bollard::network::{CreateNetworkOptions, ListNetworksOptions};
use futures_util::stream::{Stream, StreamExt};
use std::collections::HashMap;

#[async_trait]
pub trait DockerClient: Send + Sync {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<String, String>;

    async fn start_container(&self, id: &str) -> Result<(), String>;

    async fn wait_container(&self, id: &str) -> Result<i64, String>;

    #[allow(dead_code)]
    async fn logs(&self, id: &str, options: Option<LogsOptions<String>>) -> Result<String, String>;

    #[allow(dead_code)]
    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions<String>>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>;

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String>;

    async fn create_network(&self, name: &str) -> Result<String, String>;

    async fn network_exists(&self, name: &str) -> Result<bool, String>;

    /// Build a Docker image from a tar archive containing a Dockerfile and associated files with streaming output
    ///
    /// # Arguments
    /// * `options` - Build options including image tag, build args, and cache settings
    /// * `tar_archive` - Tar archive containing Dockerfile and any additional files
    ///
    /// # Returns
    /// A stream of build output messages, or an error if the build fails to start
    async fn build_image(
        &self,
        options: BuildImageOptions<String>,
        tar_archive: Vec<u8>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>;

    /// Check if a Docker image exists locally
    ///
    /// # Arguments
    /// * `tag` - The image tag to check (e.g., "tsk/rust/claude/web-api")
    ///
    /// # Returns
    /// True if the image exists, false otherwise
    async fn image_exists(&self, tag: &str) -> Result<bool, String>;
}

#[derive(Clone)]
pub struct DefaultDockerClient {
    docker: Docker,
}

impl DefaultDockerClient {
    pub fn new() -> Self {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => Self { docker },
            Err(e) => panic!(
                "Failed to connect to Docker: {e}\n\n\
                Please ensure Docker is installed and running:\n\
                  - On macOS: Open Docker Desktop application\n\
                  - On Linux: Run 'sudo systemctl start docker' or 'sudo service docker start'\n\
                  - Check Docker status with: 'docker ps'\n\n\
                If Docker is running, check permissions:\n\
                  - On Linux: Ensure your user is in the docker group: 'sudo usermod -aG docker $USER'\n\
                    - Then log out and back in for group changes to take effect"
            ),
        }
    }
}

impl Default for DefaultDockerClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DockerClient for DefaultDockerClient {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<String, String> {
        let response = self
            .docker
            .create_container(options, config)
            .await
            .map_err(|e| format!("Failed to create container: {e}"))?;
        Ok(response.id)
    }

    async fn start_container(&self, id: &str) -> Result<(), String> {
        self.docker
            .start_container::<String>(id, None)
            .await
            .map_err(|e| format!("Failed to start container: {e}"))
    }

    async fn wait_container(&self, id: &str) -> Result<i64, String> {
        use futures_util::stream::StreamExt;

        let mut stream = self
            .docker
            .wait_container(id, None::<bollard::container::WaitContainerOptions<String>>);
        if let Some(result) = stream.next().await {
            match result {
                Ok(wait_response) => Ok(wait_response.status_code),
                Err(e) => Err(format!("Failed to wait for container: {e}")),
            }
        } else {
            Err("Container wait stream ended unexpectedly".to_string())
        }
    }

    async fn logs(&self, id: &str, options: Option<LogsOptions<String>>) -> Result<String, String> {
        let mut stream = self.docker.logs(id, options);
        let mut output = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(log) => output.push_str(&log.to_string()),
                Err(e) => return Err(format!("Failed to get logs: {e}")),
            }
        }

        Ok(output)
    }

    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions<String>>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        let stream = self.docker.logs(id, options);
        let mapped_stream = stream.map(|result| match result {
            Ok(log) => Ok(log.to_string()),
            Err(e) => Err(format!("Failed to get logs: {e}")),
        });
        Ok(Box::new(Box::pin(mapped_stream)))
    }

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        self.docker
            .remove_container(id, options)
            .await
            .map_err(|e| format!("Failed to remove container: {e}"))
    }

    async fn create_network(&self, name: &str) -> Result<String, String> {
        let options = CreateNetworkOptions {
            name: name.to_string(),
            ..Default::default()
        };

        let response = self
            .docker
            .create_network(options)
            .await
            .map_err(|e| format!("Failed to create network: {e}"))?;

        Ok(response.id.unwrap_or_else(|| name.to_string()))
    }

    async fn network_exists(&self, name: &str) -> Result<bool, String> {
        let mut filters = HashMap::new();
        filters.insert("name", vec![name]);

        let options = ListNetworksOptions { filters };

        let networks = self
            .docker
            .list_networks(Some(options))
            .await
            .map_err(|e| format!("Failed to list networks: {e}"))?;

        Ok(!networks.is_empty())
    }

    async fn build_image(
        &self,
        options: BuildImageOptions<String>,
        tar_archive: Vec<u8>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>
    {
        use futures_util::StreamExt;
        use tokio::sync::mpsc;

        let (tx, rx) = mpsc::unbounded_channel();
        let docker = self.docker.clone();

        // Spawn a task to handle the streaming
        tokio::spawn(async move {
            let mut stream = docker.build_image(options, None, Some(tar_archive.into()));

            while let Some(build_info) = stream.next().await {
                match build_info {
                    Ok(info) => {
                        if let Some(error) = info.error {
                            let _ = tx.send(Err(format!("Docker build error: {error}")));
                            break;
                        } else if let Some(stream_msg) = info.stream {
                            if !stream_msg.is_empty() && tx.send(Ok(stream_msg)).is_err() {
                                break; // Receiver dropped
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Failed to build image: {e}")));
                        break;
                    }
                }
            }
        });

        // Convert receiver to stream
        let receiver_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
        Ok(Box::new(Box::pin(receiver_stream)))
    }

    async fn image_exists(&self, tag: &str) -> Result<bool, String> {
        let mut filters = HashMap::new();
        filters.insert("reference", vec![tag]);

        let options = ListImagesOptions {
            filters,
            ..Default::default()
        };

        let images = self
            .docker
            .list_images(Some(options))
            .await
            .map_err(|e| format!("Failed to list images: {e}"))?;

        Ok(!images.is_empty())
    }
}
