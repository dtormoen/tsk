pub mod docker_client;
pub mod file_system;

#[cfg(test)]
mod tests;

use docker_client::DockerClient;
use file_system::FileSystemOperations;
use std::sync::Arc;

pub struct AppContext {
    docker_client: Arc<dyn DockerClient>,
    file_system: Arc<dyn FileSystemOperations>,
}

impl AppContext {
    pub fn builder() -> AppContextBuilder {
        AppContextBuilder::new()
    }

    pub fn docker_client(&self) -> Arc<dyn DockerClient> {
        Arc::clone(&self.docker_client)
    }

    pub fn file_system(&self) -> Arc<dyn FileSystemOperations> {
        Arc::clone(&self.file_system)
    }
}

pub struct AppContextBuilder {
    docker_client: Option<Arc<dyn DockerClient>>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
}

impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            file_system: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    #[allow(dead_code)]
    pub fn with_file_system(mut self, file_system: Arc<dyn FileSystemOperations>) -> Self {
        self.file_system = Some(file_system);
        self
    }

    pub fn build(self) -> AppContext {
        AppContext {
            docker_client: self
                .docker_client
                .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new())),
            file_system: self
                .file_system
                .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem)),
        }
    }
}

#[cfg(test)]
impl AppContext {
    pub fn new_with_test_docker(docker_client: Arc<dyn DockerClient>) -> Self {
        AppContextBuilder::new()
            .with_docker_client(docker_client)
            .build()
    }
}
