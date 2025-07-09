//! Docker layer types and structures for the templating system

use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the different types of Docker layers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DockerLayerType {
    /// Base OS and essential development tools
    Base,
    /// Technology stack (e.g., rust, python, node)
    TechStack,
    /// AI agent setup (e.g., claude, aider)
    Agent,
    /// Project-specific configuration
    Project,
}

impl fmt::Display for DockerLayerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DockerLayerType::Base => write!(f, "base"),
            DockerLayerType::TechStack => write!(f, "tech-stack"),
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

#[allow(dead_code)]
impl DockerLayer {
    /// Creates a new DockerLayer
    pub fn new(layer_type: DockerLayerType, name: String) -> Self {
        Self { layer_type, name }
    }

    /// Creates a base layer
    pub fn base() -> Self {
        Self {
            layer_type: DockerLayerType::Base,
            name: "base".to_string(),
        }
    }

    /// Creates a tech stack layer
    pub fn tech_stack(name: impl Into<String>) -> Self {
        Self {
            layer_type: DockerLayerType::TechStack,
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

    /// Get the directory path for this layer in the asset structure
    pub fn asset_path(&self) -> String {
        match self.layer_type {
            DockerLayerType::Base => "base".to_string(),
            _ => format!("{}/{}", self.layer_type, self.name),
        }
    }
}

impl fmt::Display for DockerLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.layer_type, self.name)
    }
}

/// Represents the content of a Docker layer
#[derive(Debug, Clone)]
pub struct DockerLayerContent {
    /// The Dockerfile fragment for this layer
    pub dockerfile_content: String,
    /// Additional files that should be copied alongside the Dockerfile
    pub additional_files: Vec<(String, Vec<u8>)>,
}

#[allow(dead_code)]
impl DockerLayerContent {
    /// Creates a new DockerLayerContent with just Dockerfile content
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
    /// Technology stack name (e.g., "rust", "python", "node")
    pub tech_stack: String,
    /// Agent name (e.g., "claude", "aider")
    pub agent: String,
    /// Project name (e.g., "web-api", "cli-tool")
    pub project: String,
}

#[allow(dead_code)]
impl DockerImageConfig {
    /// Creates a new DockerImageConfig
    pub fn new(tech_stack: String, agent: String, project: String) -> Self {
        Self {
            tech_stack,
            agent,
            project,
        }
    }

    /// Creates a default configuration
    pub fn default_config() -> Self {
        Self {
            tech_stack: "default".to_string(),
            agent: "claude-code".to_string(),
            project: "default".to_string(),
        }
    }

    /// Generate the Docker image tag from the configuration
    pub fn image_tag(&self) -> String {
        format!("tsk/{}/{}/{}", self.tech_stack, self.agent, self.project)
    }

    /// Get all layers needed for this configuration in order
    pub fn get_layers(&self) -> Vec<DockerLayer> {
        vec![
            DockerLayer::base(),
            DockerLayer::tech_stack(&self.tech_stack),
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
        assert_eq!(base.name, "base");

        let rust = DockerLayer::tech_stack("rust");
        assert_eq!(rust.layer_type, DockerLayerType::TechStack);
        assert_eq!(rust.name, "rust");

        let claude = DockerLayer::agent("claude-code");
        assert_eq!(claude.layer_type, DockerLayerType::Agent);
        assert_eq!(claude.name, "claude-code");

        let web_api = DockerLayer::project("web-api");
        assert_eq!(web_api.layer_type, DockerLayerType::Project);
        assert_eq!(web_api.name, "web-api");
    }

    #[test]
    fn test_docker_layer_asset_path() {
        assert_eq!(DockerLayer::base().asset_path(), "base");
        assert_eq!(
            DockerLayer::tech_stack("rust").asset_path(),
            "tech-stack/rust"
        );
        assert_eq!(
            DockerLayer::agent("claude-code").asset_path(),
            "agent/claude-code"
        );
        assert_eq!(
            DockerLayer::project("web-api").asset_path(),
            "project/web-api"
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
        assert_eq!(layers[1].layer_type, DockerLayerType::TechStack);
        assert_eq!(layers[2].layer_type, DockerLayerType::Agent);
        assert_eq!(layers[3].layer_type, DockerLayerType::Project);
    }

    #[test]
    fn test_default_config() {
        let config = DockerImageConfig::default_config();
        assert_eq!(config.tech_stack, "default");
        assert_eq!(config.agent, "claude-code");
        assert_eq!(config.project, "default");
        assert_eq!(config.image_tag(), "tsk/default/claude-code/default");
    }

    #[test]
    fn test_layer_content_creation() {
        let content = DockerLayerContent::new("FROM ubuntu:22.04".to_string());
        assert_eq!(content.dockerfile_content, "FROM ubuntu:22.04");
        assert!(content.additional_files.is_empty());

        let with_files = DockerLayerContent::with_files(
            "FROM alpine".to_string(),
            vec![("config.txt".to_string(), b"test".to_vec())],
        );
        assert_eq!(with_files.dockerfile_content, "FROM alpine");
        assert_eq!(with_files.additional_files.len(), 1);
    }
}
