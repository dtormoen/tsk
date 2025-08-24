use crate::assets::layered::LayeredAssetManager;
use crate::commands::Command;
use crate::context::AppContext;
use crate::docker::composer::DockerComposer;
use crate::docker::image_manager::DockerImageManager;
use crate::docker::template_manager::DockerTemplateManager;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

/// Command to build TSK Docker images using the templating system
pub struct DockerBuildCommand {
    /// Whether to build without using Docker's cache
    pub no_cache: bool,
    /// Technology stack (defaults to "default")
    pub tech_stack: Option<String>,
    /// Agent (defaults to "claude")
    pub agent: Option<String>,
    /// Project (defaults to "default")
    pub project: Option<String>,
    /// Whether to only print the resolved Dockerfile without building
    pub dry_run: bool,
}

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Auto-detect tech_stack if not provided
        let tech_stack = match &self.tech_stack {
            Some(ts) => {
                println!("Using tech stack: {ts}");
                ts.clone()
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_tech_stack(&repo_root).await {
                    Ok(detected) => {
                        println!("Auto-detected tech stack: {detected}");
                        detected
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to detect tech stack: {e}. Using default.");
                        "default".to_string()
                    }
                }
            }
        };

        let agent = self.agent.as_deref().unwrap_or("claude-code");

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => {
                println!("Using project: {p}");
                Some(p.clone())
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_project_name(&repo_root).await {
                    Ok(detected) => {
                        println!("Auto-detected project name: {detected}");
                        Some(detected)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                        Some("default".to_string())
                    }
                }
            }
        };

        // Get project root for Docker operations
        let project_root = find_repository_root(std::path::Path::new(".")).ok();

        // Create image manager
        let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
            project_root.as_deref(),
            &ctx.tsk_config(),
        ));
        let template_manager = DockerTemplateManager::new(asset_manager.clone(), ctx.tsk_config());
        let composer =
            DockerComposer::new(DockerTemplateManager::new(asset_manager, ctx.tsk_config()));
        let image_manager =
            DockerImageManager::new(ctx.docker_client(), template_manager, composer);

        // Build the main image (with dry_run flag)
        let image = image_manager
            .build_image(
                &tech_stack,
                agent,
                project.as_deref(),
                project_root.as_deref(),
                self.no_cache,
                self.dry_run,
            )
            .await?;

        if !self.dry_run {
            println!("Successfully built Docker image: {}", image.tag);

            if image.used_fallback {
                println!(
                    "Note: Used default project layer as project-specific layer was not found"
                );
            }

            // Always build proxy image as it's still needed
            println!("\nBuilding tsk/proxy image...");
            let proxy_image = image_manager.build_proxy_image(self.no_cache).await?;
            println!("Successfully built Docker image: {}", proxy_image.tag);

            println!("\nAll Docker images built successfully!");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_build_command_creation() {
        // Test that DockerBuildCommand can be instantiated
        let _command = DockerBuildCommand {
            no_cache: false,
            tech_stack: None,
            agent: None,
            project: None,
            dry_run: false,
        };
    }

    #[test]
    fn test_docker_build_command_with_options() {
        // Test that DockerBuildCommand can be instantiated with all options
        let _command = DockerBuildCommand {
            no_cache: true,
            tech_stack: Some("rust".to_string()),
            agent: Some("claude".to_string()),
            project: Some("web-api".to_string()),
            dry_run: false,
        };
    }

    #[test]
    fn test_docker_build_command_dry_run() {
        // Test that DockerBuildCommand can be instantiated with dry_run
        let _command = DockerBuildCommand {
            no_cache: false,
            tech_stack: Some("python".to_string()),
            agent: Some("claude".to_string()),
            project: Some("test-project".to_string()),
            dry_run: true,
        };
    }
}
