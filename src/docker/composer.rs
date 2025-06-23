//! Docker layer composition engine
//!
//! This module handles the composition of multiple Docker layers into a single
//! Dockerfile, managing build arguments, environment variables, and layer ordering.

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::docker::layers::DockerImageConfig;
use crate::docker::template_manager::DockerTemplateManager;

/// Composes multiple Docker layers into a complete Dockerfile
pub struct DockerComposer {
    template_manager: DockerTemplateManager,
}

impl DockerComposer {
    /// Creates a new DockerComposer
    pub fn new(template_manager: DockerTemplateManager) -> Self {
        Self { template_manager }
    }

    /// Compose a complete Dockerfile and associated files from the given configuration
    pub fn compose(&self, config: &DockerImageConfig) -> Result<ComposedDockerfile> {
        // Get the composed Dockerfile content
        let dockerfile_content = self.template_manager.compose_dockerfile(config)?;

        // Collect all additional files from layers
        let mut additional_files = HashMap::new();
        let layers = config.get_layers();

        for layer in &layers {
            if let Ok(layer_content) = self.template_manager.get_layer_content(layer) {
                for (filename, content) in layer_content.additional_files {
                    // Later layers override earlier ones for the same filename
                    additional_files.insert(filename, content);
                }
            }
        }

        // Extract build arguments from the composed Dockerfile
        let build_args = self.extract_build_args(&dockerfile_content)?;

        Ok(ComposedDockerfile {
            dockerfile_content,
            additional_files,
            build_args,
            image_tag: config.image_tag(),
        })
    }

    /// Extract build arguments from Dockerfile content
    fn extract_build_args(&self, dockerfile_content: &str) -> Result<HashSet<String>> {
        let mut build_args = HashSet::new();

        for line in dockerfile_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("ARG ") {
                if let Some(arg_def) = trimmed.strip_prefix("ARG ") {
                    // Handle ARG NAME or ARG NAME=default_value
                    let arg_name = arg_def
                        .split_once('=')
                        .map(|(name, _)| name)
                        .unwrap_or(arg_def)
                        .trim();

                    if !arg_name.is_empty() {
                        build_args.insert(arg_name.to_string());
                    }
                }
            }
        }

        Ok(build_args)
    }

    /// Validate that a composed Dockerfile is valid
    pub fn validate_dockerfile(&self, content: &str) -> Result<()> {
        let mut has_from = false;
        let mut has_workdir = false;
        let mut has_user = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("FROM ") {
                has_from = true;
            } else if trimmed.starts_with("WORKDIR ") {
                has_workdir = true;
            } else if trimmed.starts_with("USER ") {
                has_user = true;
            }
        }

        if !has_from {
            return Err(anyhow::anyhow!(
                "Dockerfile must contain at least one FROM instruction"
            ));
        }

        if !has_workdir {
            return Err(anyhow::anyhow!(
                "Dockerfile should contain a WORKDIR instruction"
            ));
        }

        if !has_user {
            return Err(anyhow::anyhow!(
                "Dockerfile should contain a USER instruction for security"
            ));
        }

        Ok(())
    }

    /// Write composed Dockerfile and associated files to a directory
    pub fn write_to_directory(
        &self,
        composed: &ComposedDockerfile,
        output_dir: &Path,
    ) -> Result<()> {
        // Ensure directory exists
        std::fs::create_dir_all(output_dir)
            .with_context(|| format!("Failed to create output directory: {:?}", output_dir))?;

        // Write Dockerfile
        let dockerfile_path = output_dir.join("Dockerfile");
        std::fs::write(&dockerfile_path, &composed.dockerfile_content)
            .with_context(|| format!("Failed to write Dockerfile to {:?}", dockerfile_path))?;

        // Write additional files
        for (filename, content) in &composed.additional_files {
            let file_path = output_dir.join(filename);

            // Create parent directories if needed
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory for {:?}", file_path))?;
            }

            std::fs::write(&file_path, content)
                .with_context(|| format!("Failed to write file {:?}", file_path))?;
        }

        Ok(())
    }
}

/// Result of composing Docker layers
#[derive(Debug)]
pub struct ComposedDockerfile {
    /// The complete Dockerfile content
    pub dockerfile_content: String,
    /// Additional files to be placed alongside the Dockerfile
    pub additional_files: HashMap<String, Vec<u8>>,
    /// Build arguments extracted from the Dockerfile
    pub build_args: HashSet<String>,
    /// The Docker image tag for this composition
    pub image_tag: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::embedded::EmbeddedAssetManager;
    use crate::storage::xdg::XdgDirectories;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_composer() -> DockerComposer {
        let temp_dir = TempDir::new().unwrap();
        let xdg_dirs = XdgDirectories::new_with_paths(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
        );

        let template_manager =
            DockerTemplateManager::new(Arc::new(EmbeddedAssetManager), Arc::new(xdg_dirs), None);

        DockerComposer::new(template_manager)
    }

    #[test]
    fn test_extract_build_args() {
        let composer = create_test_composer();

        let dockerfile = r#"
FROM ubuntu:22.04
ARG GIT_USER_NAME
ARG GIT_USER_EMAIL=default@example.com
ARG BUILD_VERSION
RUN echo "Building..."
"#;

        let args = composer.extract_build_args(dockerfile).unwrap();
        assert!(args.contains("GIT_USER_NAME"));
        assert!(args.contains("GIT_USER_EMAIL"));
        assert!(args.contains("BUILD_VERSION"));
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn test_validate_dockerfile() {
        let composer = create_test_composer();

        // Valid Dockerfile
        let valid = r#"
FROM ubuntu:22.04
WORKDIR /workspace
USER agent
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(valid).is_ok());

        // Missing FROM
        let no_from = r#"
WORKDIR /workspace
USER agent
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(no_from).is_err());

        // Missing WORKDIR
        let no_workdir = r#"
FROM ubuntu:22.04
USER agent
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(no_workdir).is_err());

        // Missing USER
        let no_user = r#"
FROM ubuntu:22.04
WORKDIR /workspace
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(no_user).is_err());
    }

    #[test]
    fn test_write_to_directory() {
        let composer = create_test_composer();
        let temp_dir = TempDir::new().unwrap();

        let composed = ComposedDockerfile {
            dockerfile_content: "FROM ubuntu:22.04\nRUN echo 'test'".to_string(),
            additional_files: {
                let mut files = HashMap::new();
                files.insert("requirements.txt".to_string(), b"pytest==7.0.0\n".to_vec());
                files.insert("config/app.json".to_string(), b"{\"test\": true}".to_vec());
                files
            },
            build_args: HashSet::new(),
            image_tag: "tsk/test/test/test".to_string(),
        };

        composer
            .write_to_directory(&composed, temp_dir.path())
            .unwrap();

        // Check Dockerfile was written
        let dockerfile_path = temp_dir.path().join("Dockerfile");
        assert!(dockerfile_path.exists());
        let content = std::fs::read_to_string(&dockerfile_path).unwrap();
        assert!(content.contains("FROM ubuntu:22.04"));

        // Check additional files were written
        let requirements_path = temp_dir.path().join("requirements.txt");
        assert!(requirements_path.exists());
        let requirements = std::fs::read_to_string(&requirements_path).unwrap();
        assert_eq!(requirements, "pytest==7.0.0\n");

        let config_path = temp_dir.path().join("config/app.json");
        assert!(config_path.exists());
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(config, "{\"test\": true}");
    }
}
