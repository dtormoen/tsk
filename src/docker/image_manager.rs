//! Docker image management system
//!
//! This module provides centralized management for Docker images in TSK,
//! handling image selection with intelligent fallback, automated rebuilding,
//! and simplified APIs for the rest of the system.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

use crate::assets::layered::LayeredAssetManager;
use crate::context::AppContext;
use crate::docker::build_lock_manager::DockerBuildLockManager;
use crate::docker::composer::{ComposedDockerfile, DockerComposer};
use crate::docker::layers::{DockerImageConfig, DockerLayerType};
use crate::docker::template_manager::DockerTemplateManager;

/// Retrieves a git configuration value from a repository or global config.
///
/// When `build_root` is `Some`, opens the repository and reads its config,
/// which automatically includes global config as a fallback.
/// When `build_root` is `None`, uses global git config directly.
///
/// In test environments, returns default values if git config is not set,
/// avoiding the need to configure global git settings in CI.
///
/// # Arguments
/// * `build_root` - Optional path to the repository directory
/// * `key` - The git config key to retrieve (e.g., "user.name", "user.email")
///
/// # Returns
/// The configuration value as a string, or an error with instructions for configuring git.
fn get_git_config_from_repo(build_root: Option<&Path>, key: &str) -> Result<String> {
    let config_result = match build_root {
        Some(repo_path) => {
            // Open repository and get its config (includes global fallback)
            let repo = git2::Repository::open(repo_path)
                .with_context(|| format!("Failed to open repository at {}", repo_path.display()))?;
            repo.config()
                .with_context(|| "Failed to get repository config")?
                .get_string(key)
        }
        None => {
            // No repository context, use global config directly
            git2::Config::open_default()
                .with_context(|| "Failed to open global git config")?
                .get_string(key)
        }
    };

    match config_result {
        Ok(value) => Ok(value),
        Err(_) => {
            // In test environments, use default values to avoid requiring global git config
            #[cfg(test)]
            match key {
                "user.name" => return Ok("Test User".to_string()),
                "user.email" => return Ok("test@example.com".to_string()),
                _ => {}
            }

            Err(anyhow::anyhow!(
                "Git config '{}' not set. Please configure git:\n\
                 git config --global user.name \"Your Name\"\n\
                 git config --global user.email \"your@email.com\"",
                key
            ))
        }
    }
}

/// Manages Docker images for TSK
///
/// This struct provides a high-level interface for managing Docker images,
/// abstracting away the complexity of template management, layer composition,
/// and Docker build operations. It uses AppContext for dependency injection,
/// making it easy to test and configure.
pub struct DockerImageManager {
    ctx: AppContext,
    docker_build_lock_manager: Arc<DockerBuildLockManager>,
    template_manager: DockerTemplateManager,
    composer: DockerComposer,
}

impl DockerImageManager {
    /// Creates a new DockerImageManager from AppContext
    ///
    /// # Arguments
    /// * `ctx` - Application context with all dependencies
    /// * `project_root` - Optional project root for layered assets
    /// * `docker_build_lock_manager` - Optional shared lock manager; creates a new one if `None`
    pub fn new(
        ctx: &AppContext,
        project_root: Option<&std::path::Path>,
        docker_build_lock_manager: Option<Arc<DockerBuildLockManager>>,
    ) -> Self {
        let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
            project_root,
            &ctx.tsk_env(),
        ));
        let template_manager = DockerTemplateManager::new(asset_manager.clone(), ctx.tsk_env());
        let composer = DockerComposer::new(asset_manager);

        Self {
            ctx: ctx.clone(),
            docker_build_lock_manager: docker_build_lock_manager
                .unwrap_or_else(|| Arc::new(DockerBuildLockManager::new())),
            template_manager,
            composer,
        }
    }

    /// Helper to create DockerImageConfig
    fn create_config(stack: &str, agent: &str, project: &str) -> DockerImageConfig {
        DockerImageConfig::new(stack.to_string(), agent.to_string(), project.to_string())
    }

    /// Print dry run output for a composed Dockerfile
    fn print_dry_run_output(
        &self,
        composed: &ComposedDockerfile,
        stack: &str,
        agent: &str,
        project: &str,
    ) {
        println!("# Resolved Dockerfile for image: {}", composed.image_tag);
        println!("# Configuration: stack={stack}, agent={agent}, project={project}");
        println!();
        println!("{}", composed.dockerfile_content);

        if !composed.build_args.is_empty() {
            println!("\n# Build arguments:");
            for arg in &composed.build_args {
                println!("#   - {arg}");
            }
        }
    }

    /// Helper to validate layer availability
    fn validate_layers(
        &self,
        config: &DockerImageConfig,
        project_root: Option<&std::path::Path>,
    ) -> Vec<crate::docker::layers::DockerLayer> {
        config
            .get_layers()
            .into_iter()
            .filter(|layer| {
                self.template_manager
                    .get_layer_content(layer, project_root)
                    .is_err()
            })
            .collect()
    }

    /// Get the appropriate Docker image tag for the given configuration
    ///
    /// This method implements intelligent fallback:
    /// - If the project-specific layer doesn't exist and project != "default",
    ///   it will try again with project="default"
    /// - Returns error if stack or agent layers are missing
    ///
    /// # Returns
    /// A tuple of (image_tag, used_fallback) where used_fallback indicates if default was used
    fn get_image_tag(
        &self,
        stack: &str,
        agent: &str,
        project: Option<&str>,
        project_root: Option<&std::path::Path>,
    ) -> Result<(String, bool)> {
        let project = project.unwrap_or("default");
        let config = Self::create_config(stack, agent, project);
        let missing_layers = self.validate_layers(&config, project_root);

        // If only the project layer is missing and it's not "default", try fallback
        if missing_layers.len() == 1
            && missing_layers[0].layer_type == DockerLayerType::Project
            && project != "default"
        {
            println!("Note: Using default project layer as project-specific layer was not found");
            return Ok((format!("tsk/{stack}/{agent}/default"), true));
        }

        // Check for required layers - if we get here, there are missing layers
        if let Some(layer) = missing_layers.first() {
            match layer.layer_type {
                DockerLayerType::Base => {
                    return Err(anyhow::anyhow!(
                        "Base layer is missing. This is a critical error - please reinstall TSK."
                    ));
                }
                DockerLayerType::Stack => {
                    return Err(anyhow::anyhow!(
                        "Stack '{stack}' not found. Available stacks: {:?}",
                        self.template_manager
                            .list_available_layers(DockerLayerType::Stack, project_root)
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

        Ok((format!("tsk/{stack}/{agent}/{project}"), false))
    }

    /// Ensure a Docker image exists, rebuilding if necessary
    ///
    /// This method:
    /// - Checks if the image exists in the Docker daemon
    /// - If missing or force_rebuild is true, builds the image with locking
    /// - Returns the Docker image tag
    ///
    /// # Returns
    /// The Docker image tag (e.g., "tsk/rust/claude/web-api")
    pub async fn ensure_image(
        &self,
        stack: &str,
        agent: &str,
        project: Option<&str>,
        build_root: Option<&std::path::Path>,
        force_rebuild: bool,
    ) -> Result<String> {
        // Get the image tag (with fallback if needed)
        let (tag, used_fallback) = self.get_image_tag(stack, agent, project, build_root)?;

        // Check if image exists unless force rebuild
        if !force_rebuild && self.image_exists(&tag).await? {
            // Image already exists
            return Ok(tag);
        }

        // Acquire build lock for this image
        let _lock = self
            .docker_build_lock_manager
            .acquire_build_lock(&tag)
            .await;

        // Build the image
        println!("Building Docker image: {}", tag);

        // Determine actual project to use (considering fallback)
        let actual_project = if used_fallback {
            "default"
        } else {
            project.unwrap_or("default")
        };

        self.build_image(stack, agent, Some(actual_project), build_root, false, false)
            .await?;

        Ok(tag)
    }

    /// Build a Docker image for the given configuration
    ///
    /// # Arguments
    /// * `stack` - The stack layer (e.g., "rust", "python", "default")
    /// * `agent` - The agent layer (e.g., "claude")
    /// * `project` - Optional project layer (defaults to "default")
    /// * `build_root` - Optional build root directory for project-specific context
    /// * `no_cache` - Whether to build without using Docker's cache
    /// * `dry_run` - If true, only prints the composed Dockerfile without building
    ///
    /// # Returns
    /// The Docker image tag (e.g., "tsk/rust/claude/web-api")
    pub async fn build_image(
        &self,
        stack: &str,
        agent: &str,
        project: Option<&str>,
        build_root: Option<&std::path::Path>,
        no_cache: bool,
        dry_run: bool,
    ) -> Result<String> {
        let project = project.unwrap_or("default");

        // Log which repository context is being used
        match build_root {
            Some(root) => println!("Building Docker image using build root: {}", root.display()),
            None => println!("Building Docker image without project-specific context"),
        }

        // Create configuration
        let config = Self::create_config(stack, agent, project);

        // Compose the Dockerfile
        let composed = self
            .composer
            .compose(&config)
            .with_context(|| format!("Failed to compose Dockerfile for {}", config.image_tag()))?;

        // Validate the composed Dockerfile
        self.composer
            .validate_dockerfile(&composed.dockerfile_content)
            .with_context(|| "Dockerfile validation failed")?;

        if dry_run {
            self.print_dry_run_output(&composed, stack, agent, project);
        } else {
            // Normal mode: build the image
            // Get git configuration from the repository (or global config as fallback)
            let git_user_name = get_git_config_from_repo(build_root, "user.name")?;
            let git_user_email = get_git_config_from_repo(build_root, "user.email")?;

            // Get agent version if available
            let agent_version = if let Ok(agent_instance) =
                crate::agent::AgentProvider::get_agent(agent, self.ctx.tsk_env())
            {
                Some(agent_instance.version())
            } else {
                None
            };

            // Build the image
            self.build_docker_image(
                &composed,
                &git_user_name,
                &git_user_email,
                agent_version.as_deref(),
                no_cache,
                build_root,
            )
            .await?;
        }

        Ok(format!("tsk/{stack}/{agent}/{project}"))
    }

    /// Check if a Docker image exists
    async fn image_exists(&self, tag: &str) -> Result<bool> {
        // In test environments, always return true to avoid calling actual docker
        if cfg!(test) {
            return Ok(true);
        }

        self.ctx
            .docker_client()
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
        agent_version: Option<&str>,
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

        // Add agent version if provided and if ARG exists in Dockerfile
        if let Some(version) = agent_version
            && composed.build_args.contains("TSK_AGENT_VERSION")
        {
            build_args.insert("TSK_AGENT_VERSION".to_string(), version.to_string());
        }

        let mut options_builder = bollard::query_parameters::BuildImageOptionsBuilder::default();
        options_builder = options_builder.dockerfile("Dockerfile.tsk");
        options_builder = options_builder.t(&composed.image_tag);
        options_builder = options_builder.nocache(no_cache);
        options_builder = options_builder.buildargs(&build_args);
        let options = options_builder.build();

        // Build the image using the DockerClient with streaming output
        let mut build_stream = self
            .ctx
            .docker_client()
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

            // Add Dockerfile with TSK-specific name to avoid conflicts
            let dockerfile_bytes = composed.dockerfile_content.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path("Dockerfile.tsk")?;
            header.set_size(dockerfile_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, dockerfile_bytes)?;

            // Add build_root files if provided
            if let Some(build_root) = build_root {
                builder.append_dir_all(".", build_root)?;
            }

            builder.finish()?;
        }

        Ok(tar_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_manager() -> DockerImageManager {
        let ctx = AppContext::builder().build();
        DockerImageManager::new(&ctx, None, None)
    }

    #[test]
    fn test_get_image_tag_success() {
        let manager = create_test_manager();

        // Test with all default layers (should exist in embedded assets)
        let result = manager.get_image_tag("default", "claude", Some("default"), None);
        assert!(result.is_ok());

        let (tag, used_fallback) = result.unwrap();
        assert_eq!(tag, "tsk/default/claude/default");
        assert!(!used_fallback);
    }

    #[test]
    fn test_get_image_tag_fallback() {
        let manager = create_test_manager();

        // Test with non-existent project layer (should fall back to default)
        let result = manager.get_image_tag("default", "claude", Some("non-existent-project"), None);
        assert!(result.is_ok());

        let (tag, used_fallback) = result.unwrap();
        assert_eq!(tag, "tsk/default/claude/default");
        assert!(used_fallback);
    }

    #[test]
    fn test_get_image_tag_missing_stack() {
        let manager = create_test_manager();

        // Test with non-existent tech stack (should fail)
        let result = manager.get_image_tag("non-existent-stack", "claude", Some("default"), None);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Stack 'non-existent-stack' not found")
        );
    }

    #[test]
    fn test_get_image_tag_missing_agent() {
        let manager = create_test_manager();

        // Test with non-existent agent (should fail)
        let result = manager.get_image_tag("default", "non-existent-agent", Some("default"), None);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Agent 'non-existent-agent' not found")
        );
    }

    #[test]
    fn test_get_image_tag_with_none_project() {
        let manager = create_test_manager();

        // Test with None project (should use "default")
        let result = manager.get_image_tag("default", "claude", None, None);
        assert!(result.is_ok());

        let (tag, used_fallback) = result.unwrap();
        assert_eq!(tag, "tsk/default/claude/default");
        assert!(!used_fallback);
    }

    #[tokio::test]
    async fn test_build_image_modes() {
        let manager = create_test_manager();

        // Test build_image with dry_run=true
        let result = manager
            .build_image("default", "claude", Some("default"), None, false, true)
            .await;

        assert!(result.is_ok(), "Dry run failed: {:?}", result.err());
        let tag = result.unwrap();
        assert_eq!(tag, "tsk/default/claude/default");

        // Test build_image with dry_run=false (normal mode)
        let result = manager
            .build_image("default", "claude", Some("default"), None, false, false)
            .await;

        let tag = result.unwrap();
        assert_eq!(tag, "tsk/default/claude/default");
    }

    #[test]
    fn test_create_tar_archive_uses_tsk_dockerfile() {
        let manager = create_test_manager();
        let composed = ComposedDockerfile {
            dockerfile_content: "FROM ubuntu:24.04\nRUN echo 'test'".to_string(),
            build_args: std::collections::HashSet::new(),
            image_tag: "tsk/test/test/test".to_string(),
        };

        let tar_data = manager.create_tar_archive(&composed, None).unwrap();

        // Parse the tar archive to verify the Dockerfile name
        use tar::Archive;
        let mut archive = Archive::new(&tar_data[..]);
        let entries = archive.entries().unwrap();

        let mut found_dockerfile = false;
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path().unwrap();
            if path.to_str().unwrap() == "Dockerfile.tsk" {
                found_dockerfile = true;
                break;
            }
        }

        assert!(found_dockerfile, "Dockerfile.tsk not found in tar archive");
    }

    #[test]
    fn test_create_tar_archive_with_build_root() {
        let manager = create_test_manager();
        let temp_dir = TempDir::new().unwrap();

        // Create a project Dockerfile that would conflict
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM node:18").unwrap();

        let composed = ComposedDockerfile {
            dockerfile_content: "FROM ubuntu:24.04\nRUN echo 'tsk'".to_string(),
            build_args: std::collections::HashSet::new(),
            image_tag: "tsk/test/test/test".to_string(),
        };

        let tar_data = manager
            .create_tar_archive(&composed, Some(temp_dir.path()))
            .unwrap();

        // Parse the tar archive to verify both Dockerfiles exist
        use tar::Archive;
        let mut archive = Archive::new(&tar_data[..]);
        let entries = archive.entries().unwrap();

        let mut found_tsk_dockerfile = false;
        let mut found_project_dockerfile = false;
        let mut tsk_content = String::new();
        let mut project_content = String::new();

        for entry in entries {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap();
            match path.to_str().unwrap() {
                "Dockerfile.tsk" => {
                    found_tsk_dockerfile = true;
                    use std::io::Read;
                    entry.read_to_string(&mut tsk_content).unwrap();
                }
                "Dockerfile" => {
                    found_project_dockerfile = true;
                    use std::io::Read;
                    entry.read_to_string(&mut project_content).unwrap();
                }
                _ => {}
            }
        }

        assert!(
            found_tsk_dockerfile,
            "Dockerfile.tsk not found in tar archive"
        );
        assert!(
            found_project_dockerfile,
            "Project Dockerfile not found in tar archive"
        );
        assert!(
            tsk_content.contains("RUN echo 'tsk'"),
            "TSK Dockerfile has wrong content"
        );
        assert!(
            project_content.contains("FROM node:18"),
            "Project Dockerfile has wrong content"
        );
    }

    #[tokio::test]
    async fn test_agent_version_in_build_args() {
        let manager = create_test_manager();

        // Compose the Dockerfile to check if TSK_AGENT_VERSION is included
        let config = DockerImageConfig::new(
            "default".to_string(),
            "claude".to_string(),
            "default".to_string(),
        );
        let composed = manager.composer.compose(&config).unwrap();

        // Check that TSK_AGENT_VERSION is in build args
        assert!(
            composed.build_args.contains("TSK_AGENT_VERSION"),
            "TSK_AGENT_VERSION should be in build args"
        );

        // Check that the base Dockerfile contains the ARG
        assert!(
            composed
                .dockerfile_content
                .contains("ARG TSK_AGENT_VERSION"),
            "Dockerfile should contain ARG TSK_AGENT_VERSION"
        );
    }

    #[tokio::test]
    async fn test_build_image_includes_agent_version() {
        let manager = create_test_manager();

        // Build image in dry-run mode to avoid actual Docker calls
        let result = manager
            .build_image("default", "claude", Some("default"), None, false, true)
            .await;

        assert!(result.is_ok(), "Build should succeed with agent version");

        // In a real build (not dry-run), the agent version would be passed to Docker
        // This test validates that the code path compiles and executes
    }

    #[tokio::test]
    async fn test_build_lock_manager_integration() {
        use crate::docker::build_lock_manager::DockerBuildLockManager;
        use std::time::Duration;
        use tokio::time::sleep;

        // Create a shared lock manager
        let lock_manager = Arc::new(DockerBuildLockManager::new());

        // Test the lock manager directly since ensure_image won't actually build in test mode
        let lock1 = Arc::clone(&lock_manager);
        let lock2 = Arc::clone(&lock_manager);

        // Track execution order
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let order1 = Arc::clone(&order);
        let order2 = Arc::clone(&order);

        // Launch two concurrent tasks that try to acquire the same lock
        let task1 = tokio::spawn(async move {
            let _guard = lock1.acquire_build_lock("test-image").await;
            order1.lock().unwrap().push(1);
            // Hold lock for a bit
            sleep(Duration::from_millis(50)).await;
            order1.lock().unwrap().push(2);
            "task1_done"
        });

        let task2 = tokio::spawn(async move {
            // Small delay to ensure task 1 gets lock first
            sleep(Duration::from_millis(10)).await;
            let _guard = lock2.acquire_build_lock("test-image").await;
            order2.lock().unwrap().push(3);
            sleep(Duration::from_millis(10)).await;
            order2.lock().unwrap().push(4);
            "task2_done"
        });

        // Wait for both tasks
        let (result1, result2) = tokio::join!(task1, task2);

        assert_eq!(result1.unwrap(), "task1_done");
        assert_eq!(result2.unwrap(), "task2_done");

        // Check execution order - should be 1, 2, 3, 4 due to locking
        let final_order = order.lock().unwrap();
        assert_eq!(
            *final_order,
            vec![1, 2, 3, 4],
            "Tasks should execute serially due to lock"
        );
    }

    #[tokio::test]
    async fn test_parallel_ensure_image_different_images() {
        use crate::docker::build_lock_manager::DockerBuildLockManager;

        // Create a shared lock manager
        let lock_manager = Arc::new(DockerBuildLockManager::new());

        // Create managers with the same lock manager passed directly
        let ctx1 = AppContext::builder().build();
        let manager1 = DockerImageManager::new(&ctx1, None, Some(lock_manager.clone()));

        let ctx2 = AppContext::builder().build();
        let manager2 = DockerImageManager::new(&ctx2, None, Some(lock_manager.clone()));

        // Launch two concurrent ensure_image tasks for different images
        let task1 = tokio::spawn(async move {
            manager1
                .ensure_image("rust", "claude", Some("project1"), None, false)
                .await
        });

        let task2 = tokio::spawn(async move {
            manager2
                .ensure_image("python", "claude", Some("project2"), None, false)
                .await
        });

        // Both should complete without blocking each other
        let (result1, result2) = tokio::join!(task1, task2);

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Get the image tags
        let image1 = result1.unwrap().unwrap();
        let image2 = result2.unwrap().unwrap();

        // They should have different tags
        assert_ne!(image1, image2);
    }

    #[test]
    fn test_get_git_config_from_repo_with_repository() {
        use crate::test_utils::git_test_utils::TestGitRepository;

        // Create a test repository with git config
        let repo = TestGitRepository::new().unwrap();
        repo.init().unwrap();

        // Set custom user config in the repository
        repo.run_git_command(&["config", "user.name", "Custom Repo User"])
            .unwrap();
        repo.run_git_command(&["config", "user.email", "repo@example.com"])
            .unwrap();

        // Test reading config from the repository
        let name = get_git_config_from_repo(Some(repo.path()), "user.name").unwrap();
        assert_eq!(name, "Custom Repo User");

        let email = get_git_config_from_repo(Some(repo.path()), "user.email").unwrap();
        assert_eq!(email, "repo@example.com");
    }

    #[test]
    fn test_get_git_config_from_repo_without_repository() {
        // In test mode, returns test defaults if global config is not set
        let result = get_git_config_from_repo(None, "user.name");
        assert!(result.is_ok());
        let name = result.unwrap();
        assert!(!name.is_empty());

        let result = get_git_config_from_repo(None, "user.email");
        assert!(result.is_ok());
        let email = result.unwrap();
        assert!(!email.is_empty());
    }

    #[test]
    fn test_get_git_config_from_repo_unknown_key() {
        // Unknown keys should return an error even in test mode
        let result = get_git_config_from_repo(None, "user.unknown");
        assert!(result.is_err());
        // Falls through to production error message for unknown keys
        assert!(result.unwrap_err().to_string().contains("not set"));
    }
}
