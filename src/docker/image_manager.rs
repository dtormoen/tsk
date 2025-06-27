//! Docker image management system
//!
//! This module provides centralized management for Docker images in TSK,
//! handling image selection with intelligent fallback, automated rebuilding,
//! and simplified APIs for the rest of the system.

use anyhow::{Context, Result};
use bollard::image::BuildImageOptions;
use std::sync::Arc;
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
    docker_client: Arc<dyn DockerClient>,
    template_manager: DockerTemplateManager,
    composer: DockerComposer,
}

impl DockerImageManager {
    /// Creates a new DockerImageManager
    pub fn new(
        docker_client: Arc<dyn DockerClient>,
        template_manager: DockerTemplateManager,
        composer: DockerComposer,
    ) -> Self {
        Self {
            docker_client,
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
        project_root: Option<&std::path::Path>,
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
            if self
                .template_manager
                .get_layer_content(&layer, project_root)
                .is_err()
            {
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
                tag: format!("tsk/{tech_stack}/{agent}/default"),
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
                        "Technology stack '{tech_stack}' not found. Available tech stacks: {:?}",
                        self.template_manager
                            .list_available_layers(DockerLayerType::TechStack, project_root)
                    ));
                }
                DockerLayerType::Agent => {
                    return Err(anyhow::anyhow!(
                        "Agent '{agent}' not found. Available agents: {:?}",
                        self.template_manager
                            .list_available_layers(DockerLayerType::Agent, project_root)
                    ));
                }
                DockerLayerType::Project => {
                    // This should have been handled by fallback above
                    return Err(anyhow::anyhow!(
                        "Project layer '{project}' not found and default fallback failed"
                    ));
                }
            }
        }

        Ok(DockerImage {
            tag: format!("tsk/{tech_stack}/{agent}/{project}"),
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
        build_root: Option<&std::path::Path>,
        force_rebuild: bool,
    ) -> Result<DockerImage> {
        // Get the image configuration (with fallback if needed)
        let image = self.get_image(tech_stack, agent, project, build_root)?;

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

        self.build_image(tech_stack, agent, Some(actual_project), build_root, false)
            .await?;

        Ok(image)
    }

    /// Build a Docker image for the given configuration
    pub async fn build_image(
        &self,
        tech_stack: &str,
        agent: &str,
        project: Option<&str>,
        build_root: Option<&std::path::Path>,
        no_cache: bool,
    ) -> Result<DockerImage> {
        let project = project.unwrap_or("default");

        // Log which repository context is being used
        match build_root {
            Some(root) => println!("Building Docker image using build root: {}", root.display()),
            None => println!("Building Docker image without project-specific context"),
        }

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
            .compose(&config, build_root)
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
        self.build_docker_image(
            &composed,
            &git_user_name,
            &git_user_email,
            no_cache,
            build_root,
        )
        .await?;

        // Check if we used fallback
        let used_fallback = project != "default"
            && self
                .template_manager
                .get_layer_content(
                    &crate::docker::layers::DockerLayer::project(project),
                    build_root,
                )
                .is_err();

        Ok(DockerImage {
            tag: format!("tsk/{tech_stack}/{agent}/{project}"),
            used_fallback,
        })
    }

    /// Build the proxy image
    pub async fn build_proxy_image(&self, no_cache: bool) -> Result<DockerImage> {
        println!("Building proxy image: tsk/proxy");

        // Build the proxy image using the new approach
        self.build_proxy_image_internal(no_cache).await?;

        Ok(DockerImage {
            tag: "tsk/proxy".to_string(),
            used_fallback: false,
        })
    }

    /// Internal method to build the proxy image using DockerClient
    async fn build_proxy_image_internal(&self, no_cache: bool) -> Result<()> {
        use crate::assets::embedded::EmbeddedAssetManager;
        use crate::assets::utils::extract_dockerfile_to_temp;

        // Extract dockerfile to temporary directory
        let asset_manager = EmbeddedAssetManager;
        let dockerfile_dir = extract_dockerfile_to_temp(&asset_manager, "tsk-proxy")
            .context("Failed to extract proxy Dockerfile")?;

        // Create tar archive from the proxy dockerfile directory
        let tar_archive = self
            .create_tar_archive_from_directory(&dockerfile_dir)
            .context("Failed to create tar archive for proxy build")?;

        // Clean up the temporary directory
        let _ = std::fs::remove_dir_all(&dockerfile_dir);

        // Build options for proxy
        let options = BuildImageOptions {
            dockerfile: "Dockerfile".to_string(),
            t: "tsk/proxy".to_string(),
            nocache: no_cache,
            ..Default::default()
        };

        // Build the image using the DockerClient with streaming output
        let mut build_stream = self
            .docker_client
            .build_image(options, tar_archive)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build proxy image: {e}"))?;

        // Stream build output for real-time visibility
        use futures_util::StreamExt;
        while let Some(result) = build_stream.next().await {
            match result {
                Ok(line) => {
                    print!("{line}");
                    // Ensure output is flushed immediately
                    use std::io::Write;
                    std::io::stdout().flush().unwrap_or(());
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to build proxy image: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Create a tar archive from a directory
    fn create_tar_archive_from_directory(&self, dir_path: &std::path::Path) -> Result<Vec<u8>> {
        use tar::Builder;

        let mut tar_data = Vec::new();
        {
            let mut builder = Builder::new(&mut tar_data);

            // Add all files from the directory to the tar archive
            builder.append_dir_all(".", dir_path)?;

            builder.finish()?;
        }

        Ok(tar_data)
    }

    /// Check if a Docker image exists
    async fn image_exists(&self, tag: &str) -> Result<bool> {
        // In test environments, always return true to avoid calling actual docker
        if cfg!(test) {
            return Ok(true);
        }

        self.docker_client
            .image_exists(tag)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// Build a Docker image from composed content
    async fn build_docker_image(
        &self,
        composed: &ComposedDockerfile,
        git_user_name: &str,
        git_user_email: &str,
        no_cache: bool,
        build_root: Option<&std::path::Path>,
    ) -> Result<()> {
        // Create tar archive from the composed content
        let tar_archive = self
            .create_tar_archive(composed, build_root)
            .context("Failed to create tar archive for Docker build")?;

        // Prepare build options
        let mut build_args = std::collections::HashMap::new();

        // Add build arguments if they exist in the Dockerfile
        if composed.build_args.contains("GIT_USER_NAME") {
            build_args.insert("GIT_USER_NAME".to_string(), git_user_name.to_string());
        }

        if composed.build_args.contains("GIT_USER_EMAIL") {
            build_args.insert("GIT_USER_EMAIL".to_string(), git_user_email.to_string());
        }

        let options = BuildImageOptions {
            dockerfile: "Dockerfile".to_string(),
            t: composed.image_tag.clone(),
            nocache: no_cache,
            buildargs: build_args,
            ..Default::default()
        };

        // Build the image using the DockerClient with streaming output
        let mut build_stream = self
            .docker_client
            .build_image(options, tar_archive)
            .await
            .map_err(|e| anyhow::anyhow!("Docker build failed: {e}"))?;

        // Stream build output for real-time visibility
        use futures_util::StreamExt;
        while let Some(result) = build_stream.next().await {
            match result {
                Ok(line) => {
                    print!("{line}");
                    // Ensure output is flushed immediately
                    use std::io::Write;
                    std::io::stdout().flush().unwrap_or(());
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Docker build failed: {e}"));
                }
            }
        }

        Ok(())
    }

    /// Create a tar archive from composed Dockerfile content
    fn create_tar_archive(
        &self,
        composed: &ComposedDockerfile,
        build_root: Option<&std::path::Path>,
    ) -> Result<Vec<u8>> {
        use tar::Builder;

        let mut tar_data = Vec::new();
        {
            let mut builder = Builder::new(&mut tar_data);

            // Add Dockerfile
            let dockerfile_bytes = composed.dockerfile_content.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path("Dockerfile")?;
            header.set_size(dockerfile_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, dockerfile_bytes)?;

            // Add additional files
            for (filename, content) in &composed.additional_files {
                let mut header = tar::Header::new_gnu();
                header.set_path(filename)?;
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, content.as_slice())?;
            }

            // Add build_root files if provided
            if let Some(build_root) = build_root {
                builder.append_dir_all(".", build_root)?;
            }

            builder.finish()?;
        }

        Ok(tar_data)
    }
}

/// Get git configuration value
async fn get_git_config(key: &str) -> Result<String> {
    let output = ProcessCommand::new("git")
        .args(["config", "--global", key])
        .output()
        .await
        .with_context(|| format!("Failed to execute git config for {key}"))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Git config '{key}' not set. Please configure git with your name and email:\n\
             git config --global user.name \"Your Name\"\n\
             git config --global user.email \"your.email@example.com\""
        ));
    }

    let value = String::from_utf8(output.stdout)
        .context("Git config output is not valid UTF-8")?
        .trim()
        .to_string();

    if value.is_empty() {
        return Err(anyhow::anyhow!(
            "Git config '{key}' is empty. Please configure git with your name and email."
        ));
    }

    Ok(value)
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

        let template_manager =
            DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), Arc::new(xdg_dirs.clone()));

        let composer = DockerComposer::new(DockerTemplateManager::new(
            Arc::new(EmbeddedAssetManager),
            Arc::new(xdg_dirs),
        ));

        DockerImageManager::new(docker_client, template_manager, composer)
    }

    #[test]
    fn test_get_image_success() {
        let manager = create_test_manager();

        // Test with all default layers (should exist in embedded assets)
        let result = manager.get_image("default", "claude-code", Some("default"), None);
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(!image.used_fallback);
    }

    #[test]
    fn test_get_image_fallback() {
        let manager = create_test_manager();

        // Test with non-existent project layer (should fall back to default)
        let result =
            manager.get_image("default", "claude-code", Some("non-existent-project"), None);
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(image.used_fallback);
    }

    #[test]
    fn test_get_image_missing_tech_stack() {
        let manager = create_test_manager();

        // Test with non-existent tech stack (should fail)
        let result = manager.get_image("non-existent-stack", "claude-code", Some("default"), None);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Technology stack 'non-existent-stack' not found")
        );
    }

    #[test]
    fn test_get_image_missing_agent() {
        let manager = create_test_manager();

        // Test with non-existent agent (should fail)
        let result = manager.get_image("default", "non-existent-agent", Some("default"), None);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Agent 'non-existent-agent' not found")
        );
    }

    #[test]
    fn test_get_image_with_none_project() {
        let manager = create_test_manager();

        // Test with None project (should use "default")
        let result = manager.get_image("default", "claude-code", None, None);
        assert!(result.is_ok());

        let image = result.unwrap();
        assert_eq!(image.tag, "tsk/default/claude-code/default");
        assert!(!image.used_fallback);
    }
}
