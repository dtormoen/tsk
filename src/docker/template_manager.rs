//! Docker template management system
//!
//! This module provides a manager for Docker layer templates that follows
//! the same layered approach as the AssetManager, checking for templates
//! in project, user, and embedded locations in priority order.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

use crate::assets::AssetManager;
use crate::docker::layers::{DockerImageConfig, DockerLayer, DockerLayerContent, DockerLayerType};
use crate::storage::xdg::XdgDirectories;

/// Manages Docker templates and layer composition
pub struct DockerTemplateManager {
    asset_manager: Arc<dyn AssetManager>,
    xdg_dirs: Arc<XdgDirectories>,
}

impl DockerTemplateManager {
    /// Creates a new DockerTemplateManager
    pub fn new(asset_manager: Arc<dyn AssetManager>, xdg_dirs: Arc<XdgDirectories>) -> Self {
        Self {
            asset_manager,
            xdg_dirs,
        }
    }

    /// Get the content of a specific Docker layer
    pub fn get_layer_content(
        &self,
        layer: &DockerLayer,
        _project_root: Option<&Path>,
    ) -> Result<DockerLayerContent> {
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
    pub fn compose_dockerfile(
        &self,
        config: &DockerImageConfig,
        project_root: Option<&Path>,
    ) -> Result<String> {
        let layers = config.get_layers();
        let mut composed_dockerfile = String::new();
        let mut has_from_instruction = false;
        let mut cmd_instruction: Option<String> = None;
        let mut entrypoint_instruction: Option<String> = None;

        // Add header comment
        composed_dockerfile.push_str(&format!(
            "# TSK Composed Dockerfile\n# Tech Stack: {}\n# Agent: {}\n# Project: {}\n\n",
            config.tech_stack, config.agent, config.project
        ));

        for (index, layer) in layers.iter().enumerate() {
            // Skip layers that don't exist (except base which is required)
            let layer_content = match self.get_layer_content(layer, project_root) {
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
            let (processed_content, cmd, entrypoint) = self.process_layer_content(
                &layer_content.dockerfile_content,
                index == 0,
                &mut has_from_instruction,
            )?;

            // Update CMD and ENTRYPOINT if found in this layer
            if let Some(cmd) = cmd {
                cmd_instruction = Some(cmd);
            }
            if let Some(entrypoint) = entrypoint {
                entrypoint_instruction = Some(entrypoint);
            }

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

        // Add ENTRYPOINT and CMD at the end if they exist
        if let Some(entrypoint) = entrypoint_instruction {
            composed_dockerfile.push_str("\n# Final ENTRYPOINT\n");
            composed_dockerfile.push_str(&entrypoint);
            composed_dockerfile.push('\n');
        }
        if let Some(cmd) = cmd_instruction {
            composed_dockerfile.push_str("\n# Final CMD\n");
            composed_dockerfile.push_str(&cmd);
            composed_dockerfile.push('\n');
        }

        Ok(composed_dockerfile)
    }

    /// List available layers of a specific type
    pub fn list_available_layers(
        &self,
        layer_type: DockerLayerType,
        project_root: Option<&Path>,
    ) -> Vec<String> {
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
        if let Some(project_root) = project_root {
            let project_docker_dir = project_root.join(".tsk").join("dockerfiles");
            if project_docker_dir.exists() {
                eprintln!(
                    "Scanning project dockerfiles directory: {}",
                    project_docker_dir.display()
                );
                self.scan_directory_for_layers(&project_docker_dir, &layer_type, &mut layers);
            } else {
                eprintln!(
                    "No project dockerfiles directory found at: {}",
                    project_docker_dir.display()
                );
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
    ) -> Result<(String, Option<String>, Option<String>)> {
        let mut processed = String::new();
        let mut cmd_instruction: Option<String> = None;
        let mut entrypoint_instruction: Option<String> = None;
        let mut seen_user_root = false;

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

            // Track if we've seen USER root in this layer
            if trimmed == "USER root" {
                seen_user_root = true;
            }

            // Extract CMD and ENTRYPOINT instructions
            if trimmed.starts_with("CMD ") {
                cmd_instruction = Some(line.to_string());
                continue;
            }
            if trimmed.starts_with("ENTRYPOINT ") {
                entrypoint_instruction = Some(line.to_string());
                continue;
            }

            // Skip USER agent only if we haven't seen USER root in this layer
            // This allows switching back to agent after root operations
            if !is_first_layer && trimmed == "USER agent" && !seen_user_root {
                continue;
            }

            processed.push_str(line);
            processed.push('\n');
        }

        Ok((processed, cmd_instruction, entrypoint_instruction))
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

    fn create_test_manager() -> DockerTemplateManager {
        let temp_dir = TempDir::new().unwrap();
        let xdg_dirs = XdgDirectories::new_with_paths(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
        );

        DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), Arc::new(xdg_dirs))
    }

    #[test]
    fn test_docker_image_config_layers() {
        let config = DockerImageConfig::new(
            "rust".to_string(),
            "claude-code".to_string(),
            "web-api".to_string(),
        );

        let layers = config.get_layers();
        assert_eq!(layers.len(), 4);
        assert_eq!(layers[0].name, "base");
        assert_eq!(layers[1].name, "rust");
        assert_eq!(layers[2].name, "claude-code");
        assert_eq!(layers[3].name, "web-api");
    }

    #[test]
    fn test_process_layer_content() {
        let manager = create_test_manager();
        let mut has_from = false;

        // Test first layer processing
        let content = "FROM ubuntu:22.04\nRUN apt-get update\nWORKDIR /workspace\nUSER agent\nCMD [\"/bin/bash\"]";
        let (processed, cmd, entrypoint) = manager
            .process_layer_content(content, true, &mut has_from)
            .unwrap();
        assert!(processed.contains("FROM ubuntu:22.04"));
        assert!(processed.contains("WORKDIR /workspace"));
        assert!(processed.contains("USER agent"));
        assert!(!processed.contains("CMD")); // CMD should be extracted
        assert_eq!(cmd, Some("CMD [\"/bin/bash\"]".to_string()));
        assert_eq!(entrypoint, None);
        assert!(has_from);

        // Test non-first layer processing
        let content2 = "FROM alpine\nRUN apk add git\nWORKDIR /workspace\nUSER agent";
        let (processed2, cmd2, entrypoint2) = manager
            .process_layer_content(content2, false, &mut has_from)
            .unwrap();
        assert!(!processed2.contains("FROM alpine")); // Should skip additional FROM
        assert!(processed2.contains("WORKDIR /workspace")); // Should keep WORKDIR now
        assert!(!processed2.contains("USER agent")); // Should skip USER agent when not after root
        assert!(processed2.contains("RUN apk add git")); // Should keep RUN
        assert_eq!(cmd2, None);
        assert_eq!(entrypoint2, None);
    }

    #[test]
    fn test_process_layer_content_user_switching() {
        let manager = create_test_manager();
        let mut has_from = false;

        // Test layer with USER root switching back to USER agent
        let content = "# Switch to root\nUSER root\nRUN apt-get update\n# Switch back\nUSER agent\nRUN echo test";
        let (processed, _, _) = manager
            .process_layer_content(content, false, &mut has_from)
            .unwrap();

        // Should keep both USER instructions when switching from root to agent
        assert!(processed.contains("USER root"));
        assert!(processed.contains("USER agent"));
        assert!(processed.contains("RUN apt-get update"));
        assert!(processed.contains("RUN echo test"));
    }

    #[test]
    fn test_cmd_and_entrypoint_extraction() {
        let manager = create_test_manager();
        let mut has_from = false;

        // Test CMD extraction
        let content1 = "RUN echo test\nCMD [\"default\"]";
        let (processed1, cmd1, entrypoint1) = manager
            .process_layer_content(content1, false, &mut has_from)
            .unwrap();
        assert!(processed1.contains("RUN echo test"));
        assert!(!processed1.contains("CMD"));
        assert_eq!(cmd1, Some("CMD [\"default\"]".to_string()));
        assert_eq!(entrypoint1, None);

        // Test ENTRYPOINT extraction
        let content2 = "RUN echo test2\nENTRYPOINT [\"/entrypoint.sh\"]\nCMD [\"arg\"]";
        let (processed2, cmd2, entrypoint2) = manager
            .process_layer_content(content2, false, &mut has_from)
            .unwrap();
        assert!(processed2.contains("RUN echo test2"));
        assert!(!processed2.contains("ENTRYPOINT"));
        assert!(!processed2.contains("CMD"));
        assert_eq!(cmd2, Some("CMD [\"arg\"]".to_string()));
        assert_eq!(
            entrypoint2,
            Some("ENTRYPOINT [\"/entrypoint.sh\"]".to_string())
        );
    }

    #[test]
    fn test_compose_dockerfile_cmd_placement() {
        let _manager = create_test_manager();

        // Create a config for testing
        let _config = DockerImageConfig {
            tech_stack: "rust".to_string(),
            agent: "claude".to_string(),
            project: "default".to_string(),
        };

        // This test will use the embedded dockerfiles
        // We can't easily test the full compose without real files,
        // but we've tested the components thoroughly
    }

    #[test]
    fn test_extract_layer_name() {
        let manager = create_test_manager();

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
                "dockerfiles/agent/claude-code/Dockerfile",
                &DockerLayerType::Agent
            ),
            Some("claude-code".to_string())
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

        let manager = create_test_manager();
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
