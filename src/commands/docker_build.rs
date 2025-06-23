use super::Command;
use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;

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
        let image_manager = ctx.docker_image_manager();

        let tech_stack = self.tech_stack.as_deref().unwrap_or("default");
        let agent = self.agent.as_deref().unwrap_or("claude-code");
        let project = self.project.as_deref();

        if self.dry_run {
            // Dry run mode: just print the composed Dockerfile
            use crate::docker::composer::DockerComposer;
            use crate::docker::layers::DockerImageConfig;
            use crate::docker::template_manager::DockerTemplateManager;
            use crate::repo_utils::find_repository_root;

            let project_root = find_repository_root(std::path::Path::new(".")).ok();
            let template_manager = DockerTemplateManager::new(
                ctx.asset_manager(),
                ctx.xdg_directories(),
                project_root,
            );
            let composer = DockerComposer::new(template_manager);

            let config = DockerImageConfig::new(
                tech_stack.to_string(),
                agent.to_string(),
                project.unwrap_or("default").to_string(),
            );

            let composed = composer.compose(&config)?;
            composer.validate_dockerfile(&composed.dockerfile_content)?;

            println!("# Resolved Dockerfile for image: {}", composed.image_tag);
            println!(
                "# Configuration: tech_stack={}, agent={}, project={}",
                tech_stack,
                agent,
                project.unwrap_or("default")
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
            // Build the main image
            let image = image_manager
                .build_image(tech_stack, agent, project, self.no_cache)
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
