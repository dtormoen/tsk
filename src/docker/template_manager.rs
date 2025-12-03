//! Docker template management system
//!
//! This module provides a manager for Docker layer templates that follows
//! the same layered approach as the AssetManager, checking for templates
//! in project, user, and embedded locations in priority order.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

use crate::assets::AssetManager;
use crate::context::tsk_env::TskEnv;
use crate::docker::layers::{DockerLayer, DockerLayerContent, DockerLayerType};

/// Manages Docker templates and layer composition
pub struct DockerTemplateManager {
    asset_manager: Arc<dyn AssetManager>,
    tsk_env: Arc<TskEnv>,
}

impl DockerTemplateManager {
    /// Creates a new DockerTemplateManager
    pub fn new(asset_manager: Arc<dyn AssetManager>, tsk_env: Arc<TskEnv>) -> Self {
        Self {
            asset_manager,
            tsk_env,
        }
    }

    /// Get the content of a specific Docker layer (for backward compatibility)
    ///
    /// Note: This is kept for compatibility but no longer used by the composer
    pub fn get_layer_content(
        &self,
        layer: &DockerLayer,
        _project_root: Option<&Path>,
    ) -> Result<DockerLayerContent> {
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
    pub fn list_available_layers(
        &self,
        layer_type: DockerLayerType,
        project_root: Option<&Path>,
    ) -> Vec<String> {
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

        // Check user directory
        let user_docker_dir = self.tsk_env.config_dir().join("dockerfiles");
        if user_docker_dir.exists() {
            self.scan_directory_for_layers(&user_docker_dir, &layer_type, &mut layers);
        }

        // Check project directory
        if let Some(project_root) = project_root {
            let project_docker_dir = project_root.join(".tsk").join("dockerfiles");
            if project_docker_dir.exists() {
                self.scan_directory_for_layers(&project_docker_dir, &layer_type, &mut layers);
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

    /// Scan a directory for layers of a specific type using the new structure
    fn scan_directory_for_layers(
        &self,
        dir: &Path,
        layer_type: &DockerLayerType,
        layers: &mut std::collections::HashSet<String>,
    ) {
        let layer_dir = dir.join(layer_type.to_string());

        if layer_dir.exists() {
            // In the new structure, dockerfiles are directly in the layer directory
            if let Ok(entries) = std::fs::read_dir(&layer_dir) {
                for entry in entries.flatten() {
                    if let Some(filename) = entry.file_name().to_str() {
                        // Look for .dockerfile files
                        if filename.ends_with(".dockerfile")
                            && let Some(name) = filename.strip_suffix(".dockerfile")
                        {
                            layers.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::embedded::EmbeddedAssetManager;
    use crate::context::AppContext;
    use crate::docker::layers::DockerImageConfig;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_manager() -> DockerTemplateManager {
        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();

        DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), tsk_env)
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

    #[test]
    fn test_scan_directory_for_layers() {
        let temp_dir = TempDir::new().unwrap();
        let docker_dir = temp_dir.path().join("dockerfiles");

        // Create test layer directories with new structure
        fs::create_dir_all(docker_dir.join("base")).unwrap();
        fs::write(docker_dir.join("base/default.dockerfile"), "FROM ubuntu").unwrap();

        fs::create_dir_all(docker_dir.join("stack")).unwrap();
        fs::write(docker_dir.join("stack/rust.dockerfile"), "RUN install rust").unwrap();

        fs::write(
            docker_dir.join("stack/python.dockerfile"),
            "RUN install python",
        )
        .unwrap();

        let manager = create_test_manager();
        let mut layers = std::collections::HashSet::new();

        // Test base layer scanning
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::Base, &mut layers);
        assert!(layers.contains("default"));
        layers.clear();

        // Test stack layer scanning
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::Stack, &mut layers);
        assert!(layers.contains("rust"));
        assert!(layers.contains("python"));
    }

    #[test]
    fn test_scan_project_layers_with_dots() {
        let temp_dir = TempDir::new().unwrap();
        let docker_dir = temp_dir.path().join("dockerfiles");

        // Create test project layer directories with dots in names
        fs::create_dir_all(docker_dir.join("project")).unwrap();
        fs::write(
            docker_dir.join("project/test.nvim.dockerfile"),
            "FROM ubuntu\nRUN echo test.nvim",
        )
        .unwrap();
        fs::write(
            docker_dir.join("project/my.project.dockerfile"),
            "FROM ubuntu\nRUN echo my.project",
        )
        .unwrap();
        fs::write(
            docker_dir.join("project/simple-name.dockerfile"),
            "FROM ubuntu\nRUN echo simple",
        )
        .unwrap();

        let manager = create_test_manager();
        let mut layers = std::collections::HashSet::new();

        // Test project layer scanning with dots
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::Project, &mut layers);
        assert!(layers.contains("test.nvim"), "Should find test.nvim");
        assert!(layers.contains("my.project"), "Should find my.project");
        assert!(layers.contains("simple-name"), "Should find simple-name");
    }

    #[test]
    fn test_scan_project_layers_with_underscores() {
        let temp_dir = TempDir::new().unwrap();
        let docker_dir = temp_dir.path().join("dockerfiles");

        // Create test project layer directories with underscores
        fs::create_dir_all(docker_dir.join("project")).unwrap();
        fs::write(
            docker_dir.join("project/my_project.dockerfile"),
            "FROM ubuntu\nRUN echo my_project",
        )
        .unwrap();
        fs::write(
            docker_dir.join("project/test__special.dockerfile"),
            "FROM ubuntu\nRUN echo test__special",
        )
        .unwrap();

        let manager = create_test_manager();
        let mut layers = std::collections::HashSet::new();

        // Test project layer scanning with underscores
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::Project, &mut layers);
        assert!(layers.contains("my_project"), "Should find my_project");
        assert!(
            layers.contains("test__special"),
            "Should find test__special with double underscore"
        );
    }
}
