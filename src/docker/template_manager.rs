//! Docker template management system
//!
//! This module provides a manager for Docker layer templates that follows
//! the same layered approach as the AssetManager, checking for templates
//! in project, user, and embedded locations in priority order.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::assets::AssetManager;
use crate::docker::layers::{DockerImageConfig, DockerLayer, DockerLayerContent, DockerLayerType};
use crate::storage::xdg::XdgDirectories;

/// Manages Docker templates and layer composition
pub struct DockerTemplateManager {
    asset_manager: Arc<dyn AssetManager>,
    xdg_dirs: Arc<XdgDirectories>,
    project_root: Option<PathBuf>,
}

impl DockerTemplateManager {
    /// Creates a new DockerTemplateManager
    pub fn new(
        asset_manager: Arc<dyn AssetManager>,
        xdg_dirs: Arc<XdgDirectories>,
        project_root: Option<PathBuf>,
    ) -> Self {
        Self {
            asset_manager,
            xdg_dirs,
            project_root,
        }
    }

    /// Get the content of a specific Docker layer
    pub fn get_layer_content(&self, layer: &DockerLayer) -> Result<DockerLayerContent> {
        let layer_path = format!("dockerfiles/{}", layer.asset_path());

        // Try to get the Dockerfile for this layer
        let dockerfile_path = format!("{}/Dockerfile", layer_path);
        let dockerfile_content = self
            .get_docker_file_content(&dockerfile_path)
            .with_context(|| format!("Failed to get Dockerfile for layer {}", layer))?;

        // Try to get additional files if they exist
        let mut additional_files = Vec::new();

        // Check for common additional files (this could be extended or made configurable)
        let potential_files = vec![
            "requirements.txt",
            "package.json",
            "Cargo.toml",
            "config.json",
        ];

        for file_name in potential_files {
            let file_path = format!("{}/{}", layer_path, file_name);
            if let Ok(content) = self.get_docker_file_content(&file_path) {
                additional_files.push((file_name.to_string(), content));
            }
        }

        Ok(DockerLayerContent::with_files(
            String::from_utf8(dockerfile_content)?,
            additional_files,
        ))
    }

    /// Compose a complete Dockerfile from multiple layers
    pub fn compose_dockerfile(&self, config: &DockerImageConfig) -> Result<String> {
        let layers = config.get_layers();
        let mut composed_dockerfile = String::new();
        let mut has_from_instruction = false;

        // Add header comment
        composed_dockerfile.push_str(&format!(
            "# TSK Composed Dockerfile\n# Tech Stack: {}\n# Agent: {}\n# Project: {}\n\n",
            config.tech_stack, config.agent, config.project
        ));

        for (index, layer) in layers.iter().enumerate() {
            // Skip layers that don't exist (except base which is required)
            let layer_content = match self.get_layer_content(layer) {
                Ok(content) => content,
                Err(_) if layer.layer_type != DockerLayerType::Base => {
                    // Non-base layers are optional
                    continue;
                }
                Err(e) => return Err(e),
            };

            // Add layer boundary comment
            composed_dockerfile.push_str(&format!(
                "\n###### BEGIN {} LAYER: {} ######\n",
                layer.layer_type.to_string().to_uppercase(),
                layer.name
            ));

            // Process the Dockerfile content
            let processed_content = self.process_layer_content(
                &layer_content.dockerfile_content,
                index == 0,
                &mut has_from_instruction,
            )?;

            composed_dockerfile.push_str(&processed_content);
            composed_dockerfile.push('\n');

            // Add layer boundary comment
            composed_dockerfile.push_str(&format!(
                "###### END {} LAYER: {} ######\n",
                layer.layer_type.to_string().to_uppercase(),
                layer.name
            ));
        }

        // Ensure we have at least one FROM instruction
        if !has_from_instruction {
            return Err(anyhow::anyhow!(
                "No FROM instruction found in any layer. At least the base layer must contain a FROM instruction."
            ));
        }

        Ok(composed_dockerfile)
    }

    /// List available layers of a specific type
    pub fn list_available_layers(&self, layer_type: DockerLayerType) -> Vec<String> {
        let mut layers = std::collections::HashSet::new();

        let layer_dir = match layer_type {
            DockerLayerType::Base => "dockerfiles/base".to_string(),
            _ => format!("dockerfiles/{}", layer_type),
        };

        // Check embedded assets
        let dockerfiles = self.asset_manager.list_dockerfiles();
        for dockerfile in dockerfiles {
            if dockerfile.starts_with(&layer_dir) {
                if let Some(name) = self.extract_layer_name(&dockerfile, &layer_type) {
                    layers.insert(name);
                }
            }
        }

        // Check user directory
        let user_docker_dir = self.xdg_dirs.config_dir().join("dockerfiles");
        if user_docker_dir.exists() {
            self.scan_directory_for_layers(&user_docker_dir, &layer_type, &mut layers);
        }

        // Check project directory
        if let Some(ref project_root) = self.project_root {
            let project_docker_dir = project_root.join(".tsk").join("dockerfiles");
            if project_docker_dir.exists() {
                self.scan_directory_for_layers(&project_docker_dir, &layer_type, &mut layers);
            }
        }

        let mut result: Vec<String> = layers.into_iter().collect();
        result.sort();
        result
    }

    /// Process layer content to handle special cases
    fn process_layer_content(
        &self,
        content: &str,
        is_first_layer: bool,
        has_from_instruction: &mut bool,
    ) -> Result<String> {
        let mut processed = String::new();

        for line in content.lines() {
            let trimmed = line.trim();

            // Handle FROM instructions
            if trimmed.starts_with("FROM ") {
                if *has_from_instruction && !is_first_layer {
                    // Skip subsequent FROM instructions in non-base layers
                    continue;
                } else {
                    *has_from_instruction = true;
                }
            }

            // Skip lines that should only appear once
            if !is_first_layer
                && (trimmed.starts_with("WORKDIR /workspace")
                    || trimmed.starts_with("USER agent")
                    || trimmed.starts_with("CMD ")
                    || trimmed.starts_with("ENTRYPOINT "))
            {
                continue;
            }

            processed.push_str(line);
            processed.push('\n');
        }

        Ok(processed)
    }

    /// Get Docker file content from the asset manager
    fn get_docker_file_content(&self, path: &str) -> Result<Vec<u8>> {
        // For now, we'll use the existing dockerfile methods
        // In the future, this could be extended to support the new layer structure
        if let Some(dockerfile_name) = path.strip_prefix("dockerfiles/") {
            if let Some((name, file_path)) = dockerfile_name.split_once('/') {
                if file_path == "Dockerfile" {
                    self.asset_manager.get_dockerfile(name)
                } else {
                    self.asset_manager.get_dockerfile_file(name, file_path)
                }
            } else {
                self.asset_manager.get_dockerfile(dockerfile_name)
            }
        } else {
            Err(anyhow::anyhow!("Invalid dockerfile path: {}", path))
        }
    }

    /// Extract layer name from a dockerfile path
    fn extract_layer_name(&self, path: &str, layer_type: &DockerLayerType) -> Option<String> {
        let parts: Vec<&str> = path.split('/').collect();
        match layer_type {
            DockerLayerType::Base => {
                if parts.len() >= 2 && parts[1] == "base" {
                    Some("base".to_string())
                } else {
                    None
                }
            }
            _ => {
                if parts.len() >= 3 && parts[1] == layer_type.to_string() {
                    Some(parts[2].to_string())
                } else {
                    None
                }
            }
        }
    }

    /// Scan a directory for layers of a specific type
    fn scan_directory_for_layers(
        &self,
        dir: &Path,
        layer_type: &DockerLayerType,
        layers: &mut std::collections::HashSet<String>,
    ) {
        let layer_dir = match layer_type {
            DockerLayerType::Base => dir.join("base"),
            _ => dir.join(layer_type.to_string()),
        };

        if layer_dir.exists() {
            if layer_type == &DockerLayerType::Base {
                // Base layer is a special case - just check if Dockerfile exists
                if layer_dir.join("Dockerfile").exists() {
                    layers.insert("base".to_string());
                }
            } else {
                // For other layer types, each subdirectory is a layer
                if let Ok(entries) = std::fs::read_dir(&layer_dir) {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if entry.path().join("Dockerfile").exists() {
                                    layers.insert(name.to_string());
                                }
                            }
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
    use std::fs;
    use tempfile::TempDir;

    fn create_test_manager(project_root: Option<PathBuf>) -> DockerTemplateManager {
        let temp_dir = TempDir::new().unwrap();
        let xdg_dirs = XdgDirectories::new_with_paths(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
        );

        DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            Arc::new(xdg_dirs),
            project_root,
        )
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
        assert_eq!(layers[0].name, "base");
        assert_eq!(layers[1].name, "rust");
        assert_eq!(layers[2].name, "claude");
        assert_eq!(layers[3].name, "web-api");
    }

    #[test]
    fn test_process_layer_content() {
        let manager = create_test_manager(None);
        let mut has_from = false;

        // Test first layer processing
        let content = "FROM ubuntu:22.04\nRUN apt-get update\nWORKDIR /workspace\nUSER agent";
        let processed = manager
            .process_layer_content(content, true, &mut has_from)
            .unwrap();
        assert!(processed.contains("FROM ubuntu:22.04"));
        assert!(processed.contains("WORKDIR /workspace"));
        assert!(has_from);

        // Test non-first layer processing
        let content2 = "FROM alpine\nRUN apk add git\nWORKDIR /workspace\nUSER agent";
        let processed2 = manager
            .process_layer_content(content2, false, &mut has_from)
            .unwrap();
        assert!(!processed2.contains("FROM alpine")); // Should skip additional FROM
        assert!(!processed2.contains("WORKDIR /workspace")); // Should skip WORKDIR
        assert!(!processed2.contains("USER agent")); // Should skip USER
        assert!(processed2.contains("RUN apk add git")); // Should keep RUN
    }

    #[test]
    fn test_extract_layer_name() {
        let manager = create_test_manager(None);

        assert_eq!(
            manager.extract_layer_name("dockerfiles/base/Dockerfile", &DockerLayerType::Base),
            Some("base".to_string())
        );

        assert_eq!(
            manager.extract_layer_name(
                "dockerfiles/tech-stack/rust/Dockerfile",
                &DockerLayerType::TechStack
            ),
            Some("rust".to_string())
        );

        assert_eq!(
            manager.extract_layer_name(
                "dockerfiles/agent/claude/Dockerfile",
                &DockerLayerType::Agent
            ),
            Some("claude".to_string())
        );

        assert_eq!(
            manager.extract_layer_name(
                "dockerfiles/project/web-api/Dockerfile",
                &DockerLayerType::Project
            ),
            Some("web-api".to_string())
        );
    }

    #[test]
    fn test_scan_directory_for_layers() {
        let temp_dir = TempDir::new().unwrap();
        let docker_dir = temp_dir.path().join("dockerfiles");

        // Create test layer directories
        fs::create_dir_all(docker_dir.join("base")).unwrap();
        fs::write(docker_dir.join("base/Dockerfile"), "FROM ubuntu").unwrap();

        fs::create_dir_all(docker_dir.join("tech-stack/rust")).unwrap();
        fs::write(
            docker_dir.join("tech-stack/rust/Dockerfile"),
            "RUN install rust",
        )
        .unwrap();

        fs::create_dir_all(docker_dir.join("tech-stack/python")).unwrap();
        fs::write(
            docker_dir.join("tech-stack/python/Dockerfile"),
            "RUN install python",
        )
        .unwrap();

        let manager = create_test_manager(None);
        let mut layers = std::collections::HashSet::new();

        // Test base layer scanning
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::Base, &mut layers);
        assert!(layers.contains("base"));
        layers.clear();

        // Test tech-stack layer scanning
        manager.scan_directory_for_layers(&docker_dir, &DockerLayerType::TechStack, &mut layers);
        assert!(layers.contains("rust"));
        assert!(layers.contains("python"));
    }
}
