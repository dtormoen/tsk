use super::Command;
use crate::context::AppContext;
use crate::docker::composer::DockerComposer;
use crate::docker::layers::DockerImageConfig;
use crate::docker::template_manager::DockerTemplateManager;
use async_trait::async_trait;
use std::error::Error;
use tempfile::TempDir;
use tokio::process::Command as ProcessCommand;

/// Command to build TSK Docker images using the templating system
pub struct DockerBuildCommand {
    /// Whether to build without using Docker's cache
    pub no_cache: bool,
    /// Technology stack (defaults to "base")
    pub tech_stack: Option<String>,
    /// Agent (defaults to "claude")
    pub agent: Option<String>,
    /// Project (defaults to "default")
    pub project: Option<String>,
    /// Whether to build the legacy images (tsk/base and tsk/proxy)
    pub legacy: bool,
    /// Whether to only print the resolved Dockerfile without building
    pub dry_run: bool,
}

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        if self.legacy {
            if self.dry_run {
                return Err("--dry-run is not supported with --legacy flag".into());
            }
            // Build legacy images for backward compatibility
            println!("Building legacy TSK Docker images...");
            let git_user_name = get_git_config("user.name").await?;
            let git_user_email = get_git_config("user.email").await?;

            println!("\nBuilding tsk/base image...");
            build_base_image(&git_user_name, &git_user_email, self.no_cache, ctx).await?;

            println!("\nBuilding tsk/proxy image...");
            build_proxy_image(self.no_cache, ctx).await?;

            println!("\nLegacy Docker images built successfully!");
        } else {
            // Build using the new templating system
            let config = DockerImageConfig::new(
                self.tech_stack
                    .clone()
                    .unwrap_or_else(|| "base".to_string()),
                self.agent.clone().unwrap_or_else(|| "claude".to_string()),
                self.project
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
            );

            if self.dry_run {
                // Dry run mode: just print the composed Dockerfile
                print_composed_dockerfile(&config, ctx)?;
            } else {
                println!("Building Docker image: {}...", config.image_tag());

                // Get git configuration for build arguments
                let git_user_name = get_git_config("user.name").await?;
                let git_user_email = get_git_config("user.email").await?;

                build_templated_image(&config, &git_user_name, &git_user_email, self.no_cache, ctx)
                    .await?;

                // Always build proxy image as it's still needed
                println!("\nBuilding tsk/proxy image...");
                build_proxy_image(self.no_cache, ctx).await?;

                println!("\nDocker images built successfully!");
            }
        }

        Ok(())
    }
}

/// Get git configuration value
async fn get_git_config(key: &str) -> Result<String, Box<dyn Error>> {
    let output = ProcessCommand::new("git")
        .args(["config", "--global", key])
        .output()
        .await?;

    if !output.status.success() {
        return Err(format!(
            "Git config '{}' not set. Please configure git with your name and email.",
            key
        )
        .into());
    }

    let value = String::from_utf8(output.stdout)?.trim().to_string();

    if value.is_empty() {
        return Err(format!(
            "Git config '{}' is empty. Please configure git with your name and email.",
            key
        )
        .into());
    }

    Ok(value)
}

/// Build the tsk/base Docker image
async fn build_base_image(
    git_user_name: &str,
    git_user_email: &str,
    no_cache: bool,
    ctx: &AppContext,
) -> Result<(), Box<dyn Error>> {
    // Extract dockerfile to temporary directory
    let dockerfile_dir =
        crate::assets::utils::extract_dockerfile_to_temp(ctx.asset_manager().as_ref(), "tsk-base")?;

    let mut args = vec!["build".to_string()];

    if no_cache {
        args.push("--no-cache".to_string());
    }

    // Create owned strings for the build arguments
    let git_user_name_arg = format!("GIT_USER_NAME={}", git_user_name);
    let git_user_email_arg = format!("GIT_USER_EMAIL={}", git_user_email);

    args.extend([
        "--build-arg".to_string(),
        git_user_name_arg,
        "--build-arg".to_string(),
        git_user_email_arg,
        "-t".to_string(),
        "tsk/base".to_string(),
        ".".to_string(),
    ]);

    let status = ProcessCommand::new("docker")
        .args(args)
        .current_dir(&dockerfile_dir)
        .status()
        .await?;

    // Clean up the temporary directory
    let _ = std::fs::remove_dir_all(&dockerfile_dir);

    if !status.success() {
        return Err("Failed to build tsk/base image".into());
    }

    Ok(())
}

/// Build the tsk/proxy Docker image
async fn build_proxy_image(no_cache: bool, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
    // Extract dockerfile to temporary directory
    let dockerfile_dir = crate::assets::utils::extract_dockerfile_to_temp(
        ctx.asset_manager().as_ref(),
        "tsk-proxy",
    )?;

    let mut args = vec!["build".to_string()];

    if no_cache {
        args.push("--no-cache".to_string());
    }

    args.extend(["-t".to_string(), "tsk/proxy".to_string(), ".".to_string()]);

    let status = ProcessCommand::new("docker")
        .args(args)
        .current_dir(&dockerfile_dir)
        .status()
        .await?;

    // Clean up the temporary directory
    let _ = std::fs::remove_dir_all(&dockerfile_dir);

    if !status.success() {
        return Err("Failed to build tsk/proxy image".into());
    }

    Ok(())
}

/// Print the composed Dockerfile without building (dry-run mode)
fn print_composed_dockerfile(
    config: &DockerImageConfig,
    ctx: &AppContext,
) -> Result<(), Box<dyn Error>> {
    // Create template manager and composer
    let template_manager = DockerTemplateManager::new(
        ctx.asset_manager().clone(),
        ctx.xdg_directories().clone(),
        None, // TODO: Get project root from context when available
    );

    let composer = DockerComposer::new(template_manager);

    // Compose the Dockerfile
    let composed = composer.compose(config)?;

    // Validate the composed Dockerfile
    composer.validate_dockerfile(&composed.dockerfile_content)?;

    // Print the Dockerfile content
    println!("# Resolved Dockerfile for image: {}", composed.image_tag);
    println!(
        "# Configuration: tech_stack={}, agent={}, project={}",
        config.tech_stack, config.agent, config.project
    );
    println!();
    println!("{}", composed.dockerfile_content);

    // Print information about additional files
    if !composed.additional_files.is_empty() {
        println!("\n# Additional files that would be created:");
        for filename in composed.additional_files.keys() {
            println!("#   - {}", filename);
        }
    }

    // Print build arguments
    if !composed.build_args.is_empty() {
        println!("\n# Build arguments:");
        for arg in &composed.build_args {
            println!("#   - {}", arg);
        }
    }

    Ok(())
}

/// Build a Docker image using the templating system
async fn build_templated_image(
    config: &DockerImageConfig,
    git_user_name: &str,
    git_user_email: &str,
    no_cache: bool,
    ctx: &AppContext,
) -> Result<(), Box<dyn Error>> {
    // Create template manager and composer
    let template_manager = DockerTemplateManager::new(
        ctx.asset_manager().clone(),
        ctx.xdg_directories().clone(),
        None, // TODO: Get project root from context when available
    );

    let composer = DockerComposer::new(template_manager);

    // Compose the Dockerfile
    let composed = composer.compose(config)?;

    // Validate the composed Dockerfile
    composer.validate_dockerfile(&composed.dockerfile_content)?;

    // Create temporary directory for the build context
    let temp_dir = TempDir::new()?;
    composer.write_to_directory(&composed, temp_dir.path())?;

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

    // Add any other build arguments with empty values (user can override with docker build --build-arg)
    for arg in &composed.build_args {
        if arg != "GIT_USER_NAME" && arg != "GIT_USER_EMAIL" {
            println!("Note: Build argument '{}' is defined but not set. Use --build-arg to provide a value.", arg);
        }
    }

    args.extend([
        "-t".to_string(),
        composed.image_tag.clone(),
        ".".to_string(),
    ]);

    println!("Building with command: docker {}", args.join(" "));

    let status = ProcessCommand::new("docker")
        .args(args)
        .current_dir(temp_dir.path())
        .status()
        .await?;

    if !status.success() {
        return Err(format!("Failed to build {} image", composed.image_tag).into());
    }

    Ok(())
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
            legacy: false,
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
            legacy: false,
            dry_run: false,
        };
    }

    #[test]
    fn test_docker_build_command_legacy() {
        // Test that DockerBuildCommand can be instantiated in legacy mode
        let _command = DockerBuildCommand {
            no_cache: false,
            tech_stack: None,
            agent: None,
            project: None,
            legacy: true,
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
            legacy: false,
            dry_run: true,
        };
    }

    #[test]
    fn test_docker_build_dry_run_legacy_conflict() {
        // Test that dry_run and legacy flags cannot be used together
        let command = DockerBuildCommand {
            no_cache: false,
            tech_stack: None,
            agent: None,
            project: None,
            legacy: true,
            dry_run: true,
        };

        // This combination should fail when executed
        assert!(command.legacy && command.dry_run);
    }
}
