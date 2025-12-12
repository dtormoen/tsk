//! Container engine detection and configuration
//!
//! This module provides abstractions for detecting and configuring
//! container engines (Docker and Podman).

use std::env;
use std::path::PathBuf;

/// Supported container engines
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl std::fmt::Display for ContainerEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerEngine::Docker => write!(f, "docker"),
            ContainerEngine::Podman => write!(f, "podman"),
        }
    }
}

impl std::str::FromStr for ContainerEngine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "docker" => Ok(ContainerEngine::Docker),
            "podman" => Ok(ContainerEngine::Podman),
            _ => Err(format!("Unknown container engine: {s}. Use 'docker' or 'podman'.")),
        }
    }
}

/// Configuration for a container engine
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub engine: ContainerEngine,
    pub socket_path: PathBuf,
}

impl EngineConfig {
    /// Create a new engine configuration
    pub fn new(engine: ContainerEngine, socket_path: PathBuf) -> Self {
        Self { engine, socket_path }
    }
}

/// Get the default socket path for an engine
pub fn default_socket_path(engine: ContainerEngine) -> Option<PathBuf> {
    match engine {
        ContainerEngine::Podman => {
            // Try rootless first
            if let Ok(xdg_runtime) = env::var("XDG_RUNTIME_DIR") {
                let rootless_path = PathBuf::from(&xdg_runtime).join("podman/podman.sock");
                if rootless_path.exists() {
                    return Some(rootless_path);
                }
            }
            // Fall back to rootful
            let rootful_path = PathBuf::from("/var/run/podman/podman.sock");
            if rootful_path.exists() {
                return Some(rootful_path);
            }
            None
        }
        ContainerEngine::Docker => {
            let docker_path = PathBuf::from("/var/run/docker.sock");
            if docker_path.exists() {
                return Some(docker_path);
            }
            None
        }
    }
}

/// Check if an engine is available by verifying its socket exists
pub fn is_engine_available(engine: ContainerEngine) -> bool {
    default_socket_path(engine).is_some()
}

/// Detect available container engines
///
/// Returns the preferred engine (Podman first) if available,
/// or falls back to the other engine.
pub fn detect_engine() -> Option<ContainerEngine> {
    // Prefer Podman
    if is_engine_available(ContainerEngine::Podman) {
        return Some(ContainerEngine::Podman);
    }
    // Fall back to Docker
    if is_engine_available(ContainerEngine::Docker) {
        return Some(ContainerEngine::Docker);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_engine_display() {
        assert_eq!(ContainerEngine::Docker.to_string(), "docker");
        assert_eq!(ContainerEngine::Podman.to_string(), "podman");
    }

    #[test]
    fn test_container_engine_from_str() {
        assert_eq!(
            "docker".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Docker
        );
        assert_eq!(
            "podman".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Podman
        );
        assert_eq!(
            "Docker".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Docker
        );
        assert_eq!(
            "PODMAN".parse::<ContainerEngine>().unwrap(),
            ContainerEngine::Podman
        );
        assert!("invalid".parse::<ContainerEngine>().is_err());
    }

    #[test]
    fn test_engine_config_new() {
        let config = EngineConfig::new(
            ContainerEngine::Docker,
            PathBuf::from("/var/run/docker.sock"),
        );
        assert_eq!(config.engine, ContainerEngine::Docker);
        assert_eq!(config.socket_path, PathBuf::from("/var/run/docker.sock"));
    }
}
