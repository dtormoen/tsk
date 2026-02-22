//! Docker template management system
//!
//! This module provides a manager for Docker layer templates that checks
//! embedded assets for available layers.

use anyhow::{Context, Result};

use crate::assets::embedded::EmbeddedAssetManager;
use crate::docker::layers::{DockerLayer, DockerLayerContent, DockerLayerType};

/// Manages Docker templates and layer composition
pub struct DockerTemplateManager {
    asset_manager: EmbeddedAssetManager,
}

impl DockerTemplateManager {
    /// Creates a new DockerTemplateManager
    pub fn new(asset_manager: EmbeddedAssetManager) -> Self {
        Self { asset_manager }
    }

    /// Get the content of a specific Docker layer
    pub fn get_layer_content(&self, layer: &DockerLayer) -> Result<DockerLayerContent> {
        // Use the new dockerfile_path method for the simplified structure
        #[allow(deprecated)]
        let dockerfile_path = format!("dockerfiles/{}", layer.dockerfile_path());
        let dockerfile_content = self
            .get_docker_file_content(&dockerfile_path)
            .with_context(|| format!("Failed to get Dockerfile for layer {layer}"))?;

        // Additional files are no longer stored alongside dockerfiles in the new structure
        let additional_files = Vec::new();

        Ok(DockerLayerContent::with_files(
            String::from_utf8(dockerfile_content)?,
            additional_files,
        ))
    }

    /// List available layers of a specific type
    pub fn list_available_layers(&self, layer_type: DockerLayerType) -> Vec<String> {
        let mut layers = std::collections::HashSet::new();

        // Check embedded assets
        let dockerfiles = self.asset_manager.list_dockerfiles();
        for dockerfile in dockerfiles {
            // The EmbeddedAssetManager returns just directory names, so we need to check if this dockerfile
            // name matches our layer type
            if dockerfile == layer_type.to_string()
                && let Some(name) = self.extract_layer_name(&dockerfile, &layer_type)
            {
                layers.insert(name);
            }
        }

        let mut result: Vec<String> = layers.into_iter().collect();
        result.sort();
        result
    }

    /// Get Docker file content from the asset manager
    fn get_docker_file_content(&self, path: &str) -> Result<Vec<u8>> {
        // Support both old and new dockerfile paths
        if let Some(dockerfile_path) = path.strip_prefix("dockerfiles/") {
            // Try new format first: {layer_type}/{name}.dockerfile
            if dockerfile_path.ends_with(".dockerfile") {
                // Extract components for the new structure
                if let Some((layer_dir, filename)) = dockerfile_path.rsplit_once('/') {
                    // Map to old asset manager structure for now
                    if let Some(name) = filename.strip_suffix(".dockerfile") {
                        let old_path = if layer_dir == "base" && name == "default" {
                            "base".to_string()
                        } else if layer_dir == "stack" {
                            // Map stack to the new format expected by the asset manager
                            format!("stack/{name}")
                        } else {
                            format!("{layer_dir}/{name}")
                        };
                        return self.asset_manager.get_dockerfile(&old_path);
                    }
                }
            }

            // Fall back to old format handling
            if let Some((name, file_path)) = dockerfile_path.split_once('/') {
                if file_path == "Dockerfile" {
                    self.asset_manager.get_dockerfile(name)
                } else {
                    self.asset_manager.get_dockerfile_file(name, file_path)
                }
            } else {
                self.asset_manager.get_dockerfile(dockerfile_path)
            }
        } else {
            Err(anyhow::anyhow!("Invalid dockerfile path: {}", path))
        }
    }

    /// Extract layer name from a dockerfile path in the new structure
    fn extract_layer_name(&self, path: &str, layer_type: &DockerLayerType) -> Option<String> {
        // The EmbeddedAssetManager.list_dockerfiles() returns just directory names like "stack", "base", etc.
        // We need to match these against the layer type and extract layer names
        if path == layer_type.to_string() {
            // For the case where the path is just the layer type directory name,
            // we need to check what's inside that directory
            return match layer_type {
                DockerLayerType::Base => Some("default".to_string()),
                DockerLayerType::Stack => {
                    // Check if we have a "default" stack layer
                    if self.asset_manager.get_dockerfile("stack/default").is_ok() {
                        Some("default".to_string())
                    } else {
                        None
                    }
                }
                DockerLayerType::Agent => {
                    // Check if we have a "claude" agent layer
                    if self.asset_manager.get_dockerfile("agent/claude").is_ok() {
                        Some("claude".to_string())
                    } else {
                        None
                    }
                }
                DockerLayerType::Project => {
                    // Check if we have a "default" project layer
                    if self.asset_manager.get_dockerfile("project/default").is_ok() {
                        Some("default".to_string())
                    } else {
                        None
                    }
                }
            };
        }

        // Handle full paths like "dockerfiles/stack/default"
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 {
            // Map old tech-stack to new stack
            let type_str = if parts[1] == "tech-stack" {
                "stack"
            } else {
                parts[1]
            };

            if type_str == layer_type.to_string() {
                // For base layer, special case
                if layer_type == &DockerLayerType::Base && parts[1] == "base" {
                    return Some("default".to_string());
                }
                return Some(parts[2].to_string());
            }
        } else if parts.len() == 2 && parts[1] == "base" && layer_type == &DockerLayerType::Base {
            // Handle "dockerfiles/base" -> default
            return Some("default".to_string());
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::layers::DockerImageConfig;

    fn create_test_manager() -> DockerTemplateManager {
        DockerTemplateManager::new(EmbeddedAssetManager)
    }

    #[test]
    fn test_docker_image_config_layers() {
        let config = DockerImageConfig::new(
            "rust".to_string(),
            "claude".to_string(),
            "web-api".to_string(),
        );

        let layers = config.get_layers();
        assert_eq!(layers.len(), 4);
        assert_eq!(layers[0].name, "default");
        assert_eq!(layers[1].name, "rust");
        assert_eq!(layers[2].name, "claude");
        assert_eq!(layers[3].name, "web-api");
    }

    #[test]
    fn test_extract_layer_name() {
        let manager = create_test_manager();

        // Test extraction with old format (from asset manager)
        assert_eq!(
            manager.extract_layer_name("dockerfiles/base", &DockerLayerType::Base),
            Some("default".to_string())
        );

        assert_eq!(
            manager.extract_layer_name("dockerfiles/tech-stack/rust", &DockerLayerType::Stack),
            Some("rust".to_string())
        );

        assert_eq!(
            manager.extract_layer_name("dockerfiles/agent/claude", &DockerLayerType::Agent),
            Some("claude".to_string())
        );

        assert_eq!(
            manager.extract_layer_name("dockerfiles/project/web-api", &DockerLayerType::Project),
            Some("web-api".to_string())
        );
    }
}
