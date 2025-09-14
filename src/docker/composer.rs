//! Docker layer composition engine
//!
//! This module handles the composition of multiple Docker layers into a single
//! Dockerfile using template rendering.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::Arc;

use crate::assets::AssetManager;
use crate::docker::layers::DockerImageConfig;
use crate::docker::template_engine::DockerTemplateEngine;

/// Composes multiple Docker layers into a complete Dockerfile
pub struct DockerComposer {
    asset_manager: Arc<dyn AssetManager>,
}

impl DockerComposer {
    /// Creates a new DockerComposer
    pub fn new(asset_manager: Arc<dyn AssetManager>) -> Self {
        Self { asset_manager }
    }

    /// Compose a complete Dockerfile and associated files from the given configuration
    pub fn compose(&self, config: &DockerImageConfig) -> Result<ComposedDockerfile> {
        // Get the base template
        let base_dockerfile = self
            .asset_manager
            .get_dockerfile("base/default")
            .context("Failed to get base dockerfile")?;
        let base_template = String::from_utf8(base_dockerfile)
            .context("Failed to decode base dockerfile as UTF-8")?;

        // Create template engine and render the Dockerfile
        let template_engine = DockerTemplateEngine::new(&*self.asset_manager);
        let dockerfile_content = template_engine.render_dockerfile(
            &base_template,
            Some(&config.stack),
            Some(&config.agent),
            Some(&config.project),
        )?;

        // Extract build arguments from the composed Dockerfile
        let build_args = self.extract_build_args(&dockerfile_content)?;

        Ok(ComposedDockerfile {
            dockerfile_content,
            build_args,
            image_tag: config.image_tag(),
        })
    }

    /// Extract build arguments from Dockerfile content
    fn extract_build_args(&self, dockerfile_content: &str) -> Result<HashSet<String>> {
        let mut build_args = HashSet::new();

        for line in dockerfile_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("ARG ")
                && let Some(arg_def) = trimmed.strip_prefix("ARG ")
            {
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
}

/// Result of composing Docker layers
#[derive(Debug)]
pub struct ComposedDockerfile {
    /// The complete Dockerfile content
    pub dockerfile_content: String,
    /// Build arguments extracted from the Dockerfile
    pub build_args: HashSet<String>,
    /// The Docker image tag for this composition
    pub image_tag: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::embedded::EmbeddedAssetManager;
    use std::sync::Arc;

    fn create_test_composer() -> DockerComposer {
        DockerComposer::new(Arc::new(EmbeddedAssetManager::new()))
    }

    #[test]
    fn test_extract_build_args() {
        let composer = create_test_composer();

        let dockerfile = r#"
FROM ubuntu:24.04
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
FROM ubuntu:24.04
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
FROM ubuntu:24.04
USER agent
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(no_workdir).is_err());

        // Missing USER
        let no_user = r#"
FROM ubuntu:24.04
WORKDIR /workspace
RUN echo "Hello"
"#;
        assert!(composer.validate_dockerfile(no_user).is_err());
    }
}
