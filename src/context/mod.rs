pub mod docker_client;

#[cfg(test)]
mod tests;

use docker_client::DockerClient;
use std::sync::Arc;

pub struct AppContext {
    docker_client: Arc<dyn DockerClient>,
}

impl AppContext {
    pub fn new(docker_client: Arc<dyn DockerClient>) -> Self {
        Self { docker_client }
    }

    pub fn docker_client(&self) -> Arc<dyn DockerClient> {
        Arc::clone(&self.docker_client)
    }
}

#[cfg(test)]
impl AppContext {
    pub fn new_with_test_docker(docker_client: Arc<dyn DockerClient>) -> Self {
        Self { docker_client }
    }
}
