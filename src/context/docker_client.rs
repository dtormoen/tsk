use async_trait::async_trait;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use bollard::network::{CreateNetworkOptions, ListNetworksOptions};
use bollard::Docker;
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
}

#[derive(Clone)]
pub struct DefaultDockerClient {
    docker: Docker,
}

impl DefaultDockerClient {
    pub fn new() -> Self {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => Self { docker },
            Err(e) => panic!("Failed to connect to Docker: {}", e),
        }
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
            .map_err(|e| format!("Failed to create container: {}", e))?;
        Ok(response.id)
    }

    async fn start_container(&self, id: &str) -> Result<(), String> {
        self.docker
            .start_container::<String>(id, None)
            .await
            .map_err(|e| format!("Failed to start container: {}", e))
    }

    async fn wait_container(&self, id: &str) -> Result<i64, String> {
        use futures_util::stream::StreamExt;

        let mut stream = self
            .docker
            .wait_container(id, None::<bollard::container::WaitContainerOptions<String>>);
        if let Some(result) = stream.next().await {
            match result {
                Ok(wait_response) => Ok(wait_response.status_code),
                Err(e) => Err(format!("Failed to wait for container: {}", e)),
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
                Err(e) => return Err(format!("Failed to get logs: {}", e)),
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
            Err(e) => Err(format!("Failed to get logs: {}", e)),
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
            .map_err(|e| format!("Failed to remove container: {}", e))
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
            .map_err(|e| format!("Failed to create network: {}", e))?;

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
            .map_err(|e| format!("Failed to list networks: {}", e))?;

        Ok(!networks.is_empty())
    }
}
