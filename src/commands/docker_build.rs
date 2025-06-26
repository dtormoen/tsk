use super::Command;
use crate::assets::layered::LayeredAssetManager;
use crate::context::AppContext;
use crate::docker::composer::DockerComposer;
use crate::docker::image_manager::DockerImageManager;
use crate::docker::layers::DockerImageConfig;
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
                println!("Using tech stack: {}", ts);
                ts.clone()
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match ctx.repository_context().detect_tech_stack(&repo_root).await {
                    Ok(detected) => {
                        println!("Auto-detected tech stack: {}", detected);
                        detected
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to detect tech stack: {}. Using default.",
                            e
                        );
                        "default".to_string()
                    }
                }
            }
        };

        let agent = self.agent.as_deref().unwrap_or("claude-code");

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => {
                println!("Using project: {}", p);
                Some(p.clone())
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match ctx
                    .repository_context()
                    .detect_project_name(&repo_root)
                    .await
                {
                    Ok(detected) => {
                        println!("Auto-detected project name: {}", detected);
                        Some(detected)
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to detect project name: {}. Using default.",
                            e
                        );
                        Some("default".to_string())
                    }
                }
            }
        };

        // Get project root for Docker operations
        let project_root = find_repository_root(std::path::Path::new(".")).ok();

        if self.dry_run {
            // Dry run mode: just print the composed Dockerfile
            let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
                project_root.as_deref(),
                &ctx.xdg_directories(),
            ));
            let template_manager = DockerTemplateManager::new(asset_manager, ctx.xdg_directories());
            let composer = DockerComposer::new(template_manager);

            let config = DockerImageConfig::new(
                tech_stack.clone(),
                agent.to_string(),
                project.as_deref().unwrap_or("default").to_string(),
            );

            let composed = composer.compose(&config, project_root.as_deref())?;
            composer.validate_dockerfile(&composed.dockerfile_content)?;

            println!("# Resolved Dockerfile for image: {}", composed.image_tag);
            println!(
                "# Configuration: tech_stack={}, agent={}, project={}",
                tech_stack,
                agent,
                project.as_deref().unwrap_or("default")
            );
            println!();
            println!("{}", composed.dockerfile_content);

            if !composed.additional_files.is_empty() {
                println!("\n# Additional files that would be created:");
                for filename in composed.additional_files.keys() {
                    println!("#   - {}", filename);
                }
            }

            if !composed.build_args.is_empty() {
                println!("\n# Build arguments:");
                for arg in &composed.build_args {
                    println!("#   - {}", arg);
                }
            }
        } else {
            // Create image manager on-demand
            let asset_manager = Arc::new(LayeredAssetManager::new_with_standard_layers(
                project_root.as_deref(),
                &ctx.xdg_directories(),
            ));
            let template_manager =
                DockerTemplateManager::new(asset_manager.clone(), ctx.xdg_directories());
            let composer = DockerComposer::new(DockerTemplateManager::new(
                asset_manager,
                ctx.xdg_directories(),
            ));
            let image_manager =
                DockerImageManager::new(ctx.docker_client(), template_manager, composer);

            // Build the main image
            let image = image_manager
                .build_image(
                    &tech_stack,
                    agent,
                    project.as_deref(),
                    project_root.as_deref(),
                    self.no_cache,
                )
                .await?;
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
