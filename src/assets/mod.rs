//! Asset management system for TSK
//!
//! This module provides an abstraction over asset storage and retrieval,
//! allowing TSK to embed assets in the binary while maintaining flexibility
//! for future extensions like user-specific or repository-specific assets.

use anyhow::Result;
use async_trait::async_trait;

pub mod embedded;
pub mod filesystem;
pub mod layered;
pub mod utils;

/// Trait for managing TSK assets including templates and dockerfiles
#[async_trait]
pub trait AssetManager: Send + Sync {
    /// Get a template by type (e.g., "feature", "fix", "doc")
    fn get_template(&self, template_type: &str) -> Result<String>;

    /// Get a dockerfile as raw bytes
    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>>;

    /// Get a specific file from a dockerfile directory
    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>>;

    /// List all available templates
    fn list_templates(&self) -> Vec<String>;

    /// List all available dockerfiles
    fn list_dockerfiles(&self) -> Vec<String>;

    /// Get a Docker layer file
    fn get_docker_layer(
        &self,
        layer_type: &str,
        layer_name: &str,
        _project_root: Option<&std::path::Path>,
    ) -> Result<Vec<u8>> {
        // Default implementation for backward compatibility
        // Uses the existing dockerfile methods
        let dockerfile_name = if layer_type == "base" {
            "base".to_string()
        } else {
            format!("{}/{}", layer_type, layer_name)
        };
        self.get_dockerfile(&dockerfile_name)
    }

    /// List available Docker layers of a specific type
    fn list_docker_layers(
        &self,
        layer_type: &str,
        _project_root: Option<&std::path::Path>,
    ) -> Vec<String> {
        // Default implementation for backward compatibility
        let prefix = if layer_type == "base" {
            "base".to_string()
        } else {
            format!("{}/", layer_type)
        };

        self.list_dockerfiles()
            .into_iter()
            .filter(|name| name.starts_with(&prefix))
            .map(|name| {
                if layer_type == "base" {
                    "base".to_string()
                } else {
                    name.strip_prefix(&prefix).unwrap_or(&name).to_string()
                }
            })
            .collect()
    }
}
