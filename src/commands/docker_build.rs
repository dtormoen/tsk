use super::Command;
use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;
use tokio::process::Command as ProcessCommand;

/// Command to build the TSK Docker images (tsk/base and tsk/proxy)
pub struct DockerBuildCommand {
    /// Whether to build without using Docker's cache
    pub no_cache: bool,
}

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, _ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Building TSK Docker images...");

        // Get git configuration for build arguments
        let git_user_name = get_git_config("user.name").await?;
        let git_user_email = get_git_config("user.email").await?;

        // Build tsk/base image
        println!("\nBuilding tsk/base image...");
        build_base_image(&git_user_name, &git_user_email, self.no_cache).await?;

        // Build tsk/proxy image
        println!("\nBuilding tsk/proxy image...");
        build_proxy_image(self.no_cache).await?;

        println!("\nDocker images built successfully!");
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
) -> Result<(), Box<dyn Error>> {
    let dockerfile_path = Path::new("dockerfiles/tsk-base");

    if !dockerfile_path.exists() {
        return Err(format!(
            "Dockerfile directory not found: {}",
            dockerfile_path.display()
        )
        .into());
    }

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
        .current_dir(dockerfile_path)
        .status()
        .await?;

    if !status.success() {
        return Err("Failed to build tsk/base image".into());
    }

    Ok(())
}

/// Build the tsk/proxy Docker image
async fn build_proxy_image(no_cache: bool) -> Result<(), Box<dyn Error>> {
    let dockerfile_path = Path::new("dockerfiles/tsk-proxy");

    if !dockerfile_path.exists() {
        return Err(format!(
            "Dockerfile directory not found: {}",
            dockerfile_path.display()
        )
        .into());
    }

    let mut args = vec!["build".to_string()];

    if no_cache {
        args.push("--no-cache".to_string());
    }

    args.extend(["-t".to_string(), "tsk/proxy".to_string(), ".".to_string()]);

    let status = ProcessCommand::new("docker")
        .args(args)
        .current_dir(dockerfile_path)
        .status()
        .await?;

    if !status.success() {
        return Err("Failed to build tsk/proxy image".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_build_command_creation() {
        // Test that DockerBuildCommand can be instantiated
        let _command = DockerBuildCommand { no_cache: false };
    }

    #[test]
    fn test_docker_build_command_with_no_cache() {
        // Test that DockerBuildCommand can be instantiated with no_cache flag
        let _command = DockerBuildCommand { no_cache: true };
    }
}
