//! Docker layer types and structures for the templating system

use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the different types of Docker layers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DockerLayerType {
    /// Base OS and essential development tools
    Base,
    /// Technology stack (e.g., rust, python, node)
    Stack,
    /// AI agent setup (e.g., claude, codex)
    Agent,
    /// Project-specific configuration
    Project,
}

impl fmt::Display for DockerLayerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DockerLayerType::Base => write!(f, "base"),
            DockerLayerType::Stack => write!(f, "stack"),
            DockerLayerType::Agent => write!(f, "agent"),
            DockerLayerType::Project => write!(f, "project"),
        }
    }
}

/// Represents a specific Docker layer with its type and name
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DockerLayer {
    /// The type of layer
    pub layer_type: DockerLayerType,
    /// The name of the specific layer (e.g., "rust", "claude", "web-api")
    pub name: String,
}

impl DockerLayer {
    /// Creates a base layer with the default name
    pub fn base() -> Self {
        Self::base_with_name("default")
    }

    /// Creates a base layer with a specific name
    pub fn base_with_name(name: impl Into<String>) -> Self {
        Self {
            layer_type: DockerLayerType::Base,
            name: name.into(),
        }
    }

    /// Creates a stack layer
    pub fn stack(name: impl Into<String>) -> Self {
        Self {
            layer_type: DockerLayerType::Stack,
            name: name.into(),
        }
    }

    /// Creates an agent layer
    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            layer_type: DockerLayerType::Agent,
            name: name.into(),
        }
    }

    /// Creates a project layer
    pub fn project(name: impl Into<String>) -> Self {
        Self {
            layer_type: DockerLayerType::Project,
            name: name.into(),
        }
    }

    /// Get the file path for this layer's dockerfile in the new structure
    /// Returns the path relative to the dockerfiles directory
    pub fn dockerfile_path(&self) -> String {
        format!("{}/{}.dockerfile", self.layer_type, self.name)
    }
}

impl fmt::Display for DockerLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.layer_type, self.name)
    }
}

/// Represents the content of a Docker layer
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields are used via public API by template_manager
pub struct DockerLayerContent {
    /// The Dockerfile fragment for this layer
    pub dockerfile_content: String,
    /// Additional files that should be copied alongside the Dockerfile
    pub additional_files: Vec<(String, Vec<u8>)>,
}

impl DockerLayerContent {
    /// Creates a new DockerLayerContent with just Dockerfile content
    ///
    /// Used in tests and potentially useful for creating simple layer content
    /// Used in tests
    #[allow(dead_code)]
    pub fn new(dockerfile_content: String) -> Self {
        Self {
            dockerfile_content,
            additional_files: Vec::new(),
        }
    }

    /// Creates a new DockerLayerContent with Dockerfile and additional files
    pub fn with_files(
        dockerfile_content: String,
        additional_files: Vec<(String, Vec<u8>)>,
    ) -> Self {
        Self {
            dockerfile_content,
            additional_files,
        }
    }
}

/// Configuration for Docker image composition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImageConfig {
    /// Stack name (e.g., "rust", "python", "node")
    pub stack: String,
    /// Agent name (e.g., "claude", "codex")
    pub agent: String,
    /// Project name (e.g., "web-api", "cli-tool")
    pub project: String,
}

impl DockerImageConfig {
    /// Creates a new DockerImageConfig
    pub fn new(stack: String, agent: String, project: String) -> Self {
        Self {
            stack,
            agent,
            project,
        }
    }

    /// Generate the Docker image tag from the configuration
    pub fn image_tag(&self) -> String {
        format!("tsk/{}/{}/{}", self.stack, self.agent, self.project)
    }

    /// Get all layers needed for this configuration in order
    pub fn get_layers(&self) -> Vec<DockerLayer> {
        vec![
            DockerLayer::base(),
            DockerLayer::stack(&self.stack),
            DockerLayer::agent(&self.agent),
            DockerLayer::project(&self.project),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_layer_creation() {
        let base = DockerLayer::base();
        assert_eq!(base.layer_type, DockerLayerType::Base);
        assert_eq!(base.name, "default");

        let rust = DockerLayer::stack("rust");
        assert_eq!(rust.layer_type, DockerLayerType::Stack);
        assert_eq!(rust.name, "rust");

        let claude = DockerLayer::agent("claude");
        assert_eq!(claude.layer_type, DockerLayerType::Agent);
        assert_eq!(claude.name, "claude");

        let web_api = DockerLayer::project("web-api");
        assert_eq!(web_api.layer_type, DockerLayerType::Project);
        assert_eq!(web_api.name, "web-api");
    }

    #[test]
    fn test_docker_layer_dockerfile_path() {
        assert_eq!(
            DockerLayer::base().dockerfile_path(),
            "base/default.dockerfile"
        );
        assert_eq!(
            DockerLayer::stack("rust").dockerfile_path(),
            "stack/rust.dockerfile"
        );
        assert_eq!(
            DockerLayer::agent("claude").dockerfile_path(),
            "agent/claude.dockerfile"
        );
        assert_eq!(
            DockerLayer::project("web-api").dockerfile_path(),
            "project/web-api.dockerfile"
        );
    }

    #[test]
    fn test_docker_image_config() {
        let config = DockerImageConfig::new(
            "rust".to_string(),
            "claude".to_string(),
            "web-api".to_string(),
        );

        assert_eq!(config.image_tag(), "tsk/rust/claude/web-api");

        let layers = config.get_layers();
        assert_eq!(layers.len(), 4);
        assert_eq!(layers[0].layer_type, DockerLayerType::Base);
        assert_eq!(layers[1].layer_type, DockerLayerType::Stack);
        assert_eq!(layers[2].layer_type, DockerLayerType::Agent);
        assert_eq!(layers[3].layer_type, DockerLayerType::Project);
    }

    #[test]
    fn test_default_config() {
        let config = DockerImageConfig {
            stack: "default".to_string(),
            agent: "claude".to_string(),
            project: "default".to_string(),
        };
        assert_eq!(config.stack, "default");
        assert_eq!(config.agent, "claude");
        assert_eq!(config.project, "default");
        assert_eq!(config.image_tag(), "tsk/default/claude/default");
    }

    #[test]
    fn test_layer_content_creation() {
        let content = DockerLayerContent::new("FROM ubuntu:24.04".to_string());
        assert_eq!(content.dockerfile_content, "FROM ubuntu:24.04");
        assert!(content.additional_files.is_empty());

        let with_files = DockerLayerContent::with_files(
            "FROM alpine".to_string(),
            vec![("config.txt".to_string(), b"test".to_vec())],
        );
        assert_eq!(with_files.dockerfile_content, "FROM alpine");
        assert_eq!(with_files.additional_files.len(), 1);
    }
}
