//! Docker image management system
//!
//! This module provides centralized management for Docker images in TSK,
//! handling image selection with intelligent fallback, automated rebuilding,
//! and simplified APIs for the rest of the system.

use anyhow::{Context, Result};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::process::Command as ProcessCommand;

use crate::context::docker_client::DockerClient;
use crate::docker::composer::{ComposedDockerfile, DockerComposer};
use crate::docker::layers::{DockerImageConfig, DockerLayerType};
use crate::docker::template_manager::DockerTemplateManager;

/// Information about a Docker image
#[derive(Debug, Clone)]
pub struct DockerImage {
    /// The full image tag (e.g., "tsk/rust/claude/web-api")
    pub tag: String,
    /// Whether this image used fallback (project layer was missing)
    pub used_fallback: bool,
}

/// Manages Docker images for TSK
pub struct DockerImageManager {
    template_manager: DockerTemplateManager,
    composer: DockerComposer,
}

impl DockerImageManager {
    /// Creates a new DockerImageManager
    pub fn new(
        _docker_client: Arc<dyn DockerClient>,
        template_manager: DockerTemplateManager,
        composer: DockerComposer,
    ) -> Self {
        Self {
            template_manager,
            composer,
        }
    }

    /// Get the appropriate Docker image for the given configuration
    ///
    /// This method implements intelligent fallback:
    /// - If the project-specific layer doesn't exist and project != "default",
    ///   it will try again with project="default"
    /// - Returns error if tech_stack or agent layers are missing
    pub fn get_image(
        &self,
        tech_stack: &str,
        agent: &str,
        project: Option<&str>,
    ) -> Result<DockerImage> {
        let project = project.unwrap_or("default");

        // Use agent name directly for dockerfile directories
        let dockerfile_agent = agent;

        // Create config with the requested layers
        let config = DockerImageConfig::new(
            tech_stack.to_string(),
            dockerfile_agent.to_string(),
            project.to_string(),
        );

        // Check if all layers exist
        let mut missing_layers = Vec::new();
        for layer in config.get_layers() {
            if self.template_manager.get_layer_content(&layer).is_err() {
                missing_layers.push(layer);
            }
        }

        // If only the project layer is missing and it's not "default", try fallback
        if missing_layers.len() == 1
            && missing_layers[0].layer_type == DockerLayerType::Project
            && project != "default"
        {
            // Project layer not found, falling back to 'default'

            // Try with default project
            let _fallback_config = DockerImageConfig::new(
                tech_stack.to_string(),
                dockerfile_agent.to_string(),
                "default".to_string(),
            );

            return Ok(DockerImage {
                tag: format!("tsk/{}/{}/default", tech_stack, agent),
                used_fallback: true,
            });
        }

        // Check for required layers - if we get here, there are missing layers
        if let Some(layer) = missing_layers.first() {
            match layer.layer_type {
                DockerLayerType::Base => {
                    return Err(anyhow::anyhow!(
                        "Base layer is missing. This is a critical error - please reinstall TSK."
                    ));
                }
                DockerLayerType::TechStack => {
                    return Err(anyhow::anyhow!(
                        "Technology stack '{}' not found. Available tech stacks: {:?}",
                        tech_stack,
                        self.template_manager
                            .list_available_layers(DockerLayerType::TechStack)
                    ));
                }
                DockerLayerType::Agent => {
                    return Err(anyhow::anyhow!(
                        "Agent '{}' not found. Available agents: {:?}",
                        agent,
                        self.template_manager
                            .list_available_layers(DockerLayerType::Agent)
                    ));
                }
                DockerLayerType::Project => {
                    // This should have been handled by fallback above
                    return Err(anyhow::anyhow!(
                        "Project layer '{}' not found and default fallback failed",
                        project
                    ));
                }
            }
        }

        Ok(DockerImage {
            tag: format!("tsk/{}/{}/{}", tech_stack, agent, project),
            used_fallback: false,
        })
    }

    /// Ensure a Docker image exists, rebuilding if necessary
    ///
    /// This method:
    /// - Checks if the image exists in the Docker daemon
    /// - If missing or force_rebuild is true, builds the image
    /// - Returns the DockerImage information
    pub async fn ensure_image(
        &self,
        tech_stack: &str,
        agent: &str,
        project: Option<&str>,
        force_rebuild: bool,
    ) -> Result<DockerImage> {
        // Get the image configuration (with fallback if needed)
        let image = self.get_image(tech_stack, agent, project)?;

        // Check if image exists unless force rebuild
        if !force_rebuild && self.image_exists(&image.tag).await? {
            // Image already exists
            return Ok(image);
        }

        // Build the image
        println!("Building Docker image: {}", image.tag);

        // Determine actual project to use (considering fallback)
        let actual_project = if image.used_fallback {
            "default"
        } else {
            project.unwrap_or("default")
        };

        self.build_image(tech_stack, agent, Some(actual_project), false)
            .await?;

        Ok(image)
    }

    /// Build a Docker image for the given configuration
    pub async fn build_image(
        &self,
        tech_stack: &str,
        agent: &str,
        project: Option<&str>,
        no_cache: bool,
    ) -> Result<DockerImage> {
        let project = project.unwrap_or("default");

        // Use agent name directly for dockerfile directories
        let dockerfile_agent = agent;

        // Create configuration
        let config = DockerImageConfig::new(
            tech_stack.to_string(),
            dockerfile_agent.to_string(),
            project.to_string(),
        );

        // Compose the Dockerfile
        let composed = self
            .composer
            .compose(&config)
            .with_context(|| format!("Failed to compose Dockerfile for {}", config.image_tag()))?;

        // Validate the composed Dockerfile
        self.composer
            .validate_dockerfile(&composed.dockerfile_content)
            .with_context(|| "Dockerfile validation failed")?;

        // Get git configuration for build arguments
        let git_user_name = get_git_config("user.name")
            .await
            .context("Failed to get git user.name")?;
        let git_user_email = get_git_config("user.email")
            .await
            .context("Failed to get git user.email")?;

        // Build the image
        self.build_docker_image(&composed, &git_user_name, &git_user_email, no_cache)
            .await?;

        // Check if we used fallback
        let used_fallback = project != "default"
            && self
                .template_manager
                .get_layer_content(&crate::docker::layers::DockerLayer::project(project))
                .is_err();

        Ok(DockerImage {
            tag: format!("tsk/{}/{}/{}", tech_stack, agent, project),
            used_fallback,
        })
    }

    /// Build the proxy image
    pub async fn build_proxy_image(&self, no_cache: bool) -> Result<DockerImage> {
        println!("Building proxy image: tsk/proxy");

        // For now, use the legacy build approach for proxy
        // In the future, this could be converted to use the layer system
        build_proxy_image_legacy(no_cache).await?;

        Ok(DockerImage {
            tag: "tsk/proxy".to_string(),
            used_fallback: false,
        })
    }

    /// Check if a Docker image exists
    async fn image_exists(&self, tag: &str) -> Result<bool> {
        // In test environments, always return true to avoid calling actual docker
        if cfg!(test) {
            return Ok(true);
        }

        let output = ProcessCommand::new("docker")
            .args(["images", "-q", tag])
            .output()
            .await
            .context("Failed to check if Docker image exists")?;

        Ok(!output.stdout.is_empty())
    }

    /// Build a Docker image from composed content
    async fn build_docker_image(
        &self,
        composed: &ComposedDockerfile,
        git_user_name: &str,
        git_user_email: &str,
        no_cache: bool,
    ) -> Result<()> {
        // Create temporary directory for the build context
        let temp_dir =
            TempDir::new().context("Failed to create temporary directory for Docker build")?;

        self.composer
            .write_to_directory(composed, temp_dir.path())
            .context("Failed to write Dockerfile to temporary directory")?;

        // Build the Docker image
        let mut args = vec!["build".to_string()];

        if no_cache {
            args.push("--no-cache".to_string());
        }

        // Add build arguments
        if composed.build_args.contains("GIT_USER_NAME") {
            args.extend([
                "--build-arg".to_string(),
                format!("GIT_USER_NAME={}", git_user_name),
            ]);
        }

        if composed.build_args.contains("GIT_USER_EMAIL") {
            args.extend([
                "--build-arg".to_string(),
                format!("GIT_USER_EMAIL={}", git_user_email),
            ]);
        }

        args.extend([
            "-t".to_string(),
            composed.image_tag.clone(),
            ".".to_string(),
        ]);

        let status = ProcessCommand::new("docker")
            .args(&args)
            .current_dir(temp_dir.path())
            .status()
            .await
            .context("Failed to execute docker build command")?;

        if !status.success() {
            return Err(anyhow::anyhow!(
                "Docker build failed for image {}",
                composed.image_tag
            ));
        }

        Ok(())
    }
}

/// Get git configuration value
async fn get_git_config(key: &str) -> Result<String> {
    let output = ProcessCommand::new("git")
        .args(["config", "--global", key])
        .output()
        .await
        .with_context(|| format!("Failed to execute git config for {}", key))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Git config '{}' not set. Please configure git with your name and email:\n\
             git config --global user.name \"Your Name\"\n\
             git config --global user.email \"your.email@example.com\"",
            key
        ));
    }

    let value = String::from_utf8(output.stdout)
        .context("Git config output is not valid UTF-8")?
        .trim()
        .to_string();

    if value.is_empty() {
        return Err(anyhow::anyhow!(
            "Git config '{}' is empty. Please configure git with your name and email.",
            key
        ));
    }

    Ok(value)
}

/// Build the proxy image
async fn build_proxy_image_legacy(no_cache: bool) -> Result<()> {
    use crate::assets::embedded::EmbeddedAssetManager;

    // Extract dockerfile to temporary directory
    let asset_manager = EmbeddedAssetManager;
    let dockerfile_dir =
        crate::assets::utils::extract_dockerfile_to_temp(&asset_manager, "tsk-proxy")
            .context("Failed to extract proxy Dockerfile")?;

    let mut args = vec!["build".to_string()];

    if no_cache {
        args.push("--no-cache".to_string());
    }

    args.extend(["-t".to_string(), "tsk/proxy".to_string(), ".".to_string()]);

    let status = ProcessCommand::new("docker")
        .args(args)
        .current_dir(&dockerfile_dir)
        .status()
        .await
        .context("Failed to execute docker build command for proxy")?;

    // Clean up the temporary directory
    let _ = std::fs::remove_dir_all(&dockerfile_dir);

    if !status.success() {
        return Err(anyhow::anyhow!("Failed to build tsk/proxy image"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::embedded::EmbeddedAssetManager;
    use crate::test_utils::TrackedDockerClient;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_manager() -> DockerImageManager {
        let docker_client = Arc::new(TrackedDockerClient::default());
        let temp_dir = TempDir::new().unwrap();
        let xdg_dirs = crate::storage::xdg::XdgDirectories::new_with_paths(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
        );

        let template_manager = DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            Arc::new(xdg_dirs.clone()),
            None,
        );

        let composer = DockerComposer::new(DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            Arc::new(xdg_dirs),
            None,
        ));

        DockerImageManager::new(docker_client, template_manager, composer)
    }

    #[test]
    fn test_get_image_success() {
        let manager = create_test_manager();

        // Test with all default layers (should exist in embedded assets)
        let result = manager.get_image("default", "claude-code", Some("default"));
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(!image.used_fallback);
    }

    #[test]
    fn test_get_image_fallback() {
        let manager = create_test_manager();

        // Test with non-existent project layer (should fall back to default)
        let result = manager.get_image("default", "claude-code", Some("non-existent-project"));
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(image.used_fallback);
    }

    #[test]
    fn test_get_image_missing_tech_stack() {
        let manager = create_test_manager();

        // Test with non-existent tech stack (should fail)
        let result = manager.get_image("non-existent-stack", "claude-code", Some("default"));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Technology stack 'non-existent-stack' not found"));
    }

    #[test]
    fn test_get_image_missing_agent() {
        let manager = create_test_manager();

        // Test with non-existent agent (should fail)
        let result = manager.get_image("default", "non-existent-agent", Some("default"));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Agent 'non-existent-agent' not found"));
    }

    #[test]
    fn test_get_image_with_none_project() {
        let manager = create_test_manager();

        // Test with None project (should use "default")
        let result = manager.get_image("default", "claude-code", None);
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(!image.used_fallback);
    }
}
