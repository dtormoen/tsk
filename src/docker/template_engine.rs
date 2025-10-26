use anyhow::{Context, Result};
use handlebars::Handlebars;
use std::collections::HashMap;

use crate::assets::AssetManager;

/// Template engine for rendering Dockerfiles using Handlebars templates
pub struct DockerTemplateEngine<'a> {
    handlebars: Handlebars<'a>,
    asset_manager: &'a dyn AssetManager,
}

impl<'a> DockerTemplateEngine<'a> {
    /// Creates a new Docker template engine
    pub fn new(asset_manager: &'a dyn AssetManager) -> Self {
        Self {
            handlebars: Handlebars::new(),
            asset_manager,
        }
    }

    /// Renders a Dockerfile by replacing template placeholders with layer contents
    pub fn render_dockerfile(
        &self,
        base_template: &str,
        stack: Option<&str>,
        agent: Option<&str>,
        project: Option<&str>,
    ) -> Result<String> {
        let mut context = HashMap::new();

        // Get layer contents, falling back to empty string if not found
        context.insert("STACK", self.get_layer_content("stack", stack)?);
        context.insert("AGENT", self.get_layer_content("agent", agent)?);
        context.insert("PROJECT", self.get_layer_content("project", project)?);

        self.handlebars
            .render_template(base_template, &context)
            .context("Failed to render Dockerfile template")
    }

    /// Gets the content of a layer snippet from the asset manager
    fn get_layer_content(&self, layer_type: &str, layer_name: Option<&str>) -> Result<String> {
        match layer_name {
            Some(name) => {
                let path = format!("{}/{}", layer_type, name);
                match self.asset_manager.get_dockerfile(&path) {
                    Ok(content) => String::from_utf8(content)
                        .context("Failed to decode layer content as UTF-8"),
                    Err(_) => {
                        // Fall back to empty string if layer not found
                        Ok(String::new())
                    }
                }
            }
            None => Ok(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::embedded::EmbeddedAssetManager;

    #[test]
    fn test_render_dockerfile_with_all_layers() {
        let asset_manager = EmbeddedAssetManager::new();
        let engine = DockerTemplateEngine::new(&asset_manager);

        let base_template = r#"FROM ubuntu:24.04
# Stack layer
{{{STACK}}}
# End of Stack layer

# Agent layer
{{{AGENT}}}
# End of Agent layer

# Project layer
{{{PROJECT}}}
# End of Project layer
"#;

        // Note: This test will use actual embedded assets if they exist
        let result = engine
            .render_dockerfile(base_template, Some("rust"), Some("claude"), Some("default"))
            .unwrap();

        assert!(result.contains("FROM ubuntu:24.04"));
        assert!(result.contains("# Stack layer"));
        assert!(result.contains("# Agent layer"));
        assert!(result.contains("# Project layer"));
    }

    #[test]
    fn test_render_dockerfile_with_missing_layers() {
        let asset_manager = EmbeddedAssetManager::new();
        let engine = DockerTemplateEngine::new(&asset_manager);

        let base_template = r#"FROM ubuntu:24.04
{{{STACK}}}
{{{AGENT}}}
{{{PROJECT}}}
"#;

        // Use non-existent layer names
        let result = engine
            .render_dockerfile(
                base_template,
                Some("nonexistent"),
                Some("missing"),
                Some("notfound"),
            )
            .unwrap();

        // Should still render with empty content for missing layers
        assert_eq!(result, "FROM ubuntu:24.04\n\n\n\n");
    }

    #[test]
    fn test_render_dockerfile_with_no_layers() {
        let asset_manager = EmbeddedAssetManager::new();
        let engine = DockerTemplateEngine::new(&asset_manager);

        let base_template = r#"FROM ubuntu:24.04
{{{STACK}}}
{{{AGENT}}}
{{{PROJECT}}}
CMD ["/bin/bash"]"#;

        let result = engine
            .render_dockerfile(base_template, None, None, None)
            .unwrap();

        assert_eq!(result, "FROM ubuntu:24.04\n\n\n\nCMD [\"/bin/bash\"]");
    }

    #[test]
    fn test_no_html_escaping_in_layers() {
        // Create a mock asset manager that returns content with special characters
        use std::collections::HashMap;

        struct MockAssetManager {
            assets: HashMap<String, Vec<u8>>,
        }

        impl AssetManager for MockAssetManager {
            fn get_template(&self, _template_type: &str) -> anyhow::Result<String> {
                Err(anyhow::anyhow!("Templates not supported in mock"))
            }

            fn get_dockerfile(&self, path: &str) -> anyhow::Result<Vec<u8>> {
                self.assets
                    .get(path)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Asset not found: {}", path))
            }

            fn get_dockerfile_file(
                &self,
                _dockerfile_name: &str,
                _file_path: &str,
            ) -> anyhow::Result<Vec<u8>> {
                Err(anyhow::anyhow!("Dockerfile files not supported in mock"))
            }

            fn list_templates(&self) -> Vec<String> {
                vec![]
            }

            fn list_dockerfiles(&self) -> Vec<String> {
                self.assets.keys().cloned().collect()
            }
        }

        let mut assets = HashMap::new();
        // Add a stack layer with special characters that would be escaped in HTML
        assets.insert(
            "stack/test".to_string(),
            r#"RUN curl --proto '=https' --tlsv1.2 -sSf https://example.com | sh && \
    echo "PATH=/usr/local/bin:$PATH" >> ~/.bashrc"#
                .as_bytes()
                .to_vec(),
        );

        let asset_manager = MockAssetManager { assets };
        let engine = DockerTemplateEngine::new(&asset_manager);

        let base_template = r#"FROM ubuntu:24.04
# Stack layer
{{{STACK}}}
# End of Stack layer"#;

        let result = engine
            .render_dockerfile(base_template, Some("test"), None, None)
            .unwrap();

        // Verify that special characters are NOT escaped
        assert!(
            result.contains("--proto '=https'"),
            "Single quotes should not be escaped"
        );
        assert!(
            result.contains(r#"echo "PATH="#),
            "Double quotes should not be escaped"
        );
        assert!(result.contains("&&"), "Ampersands should not be escaped");
        assert!(
            !result.contains("&#x27;"),
            "Should not contain HTML entity for single quote"
        );
        assert!(
            !result.contains("&#x3D;"),
            "Should not contain HTML entity for equals sign"
        );
        assert!(
            !result.contains("&quot;"),
            "Should not contain HTML entity for double quote"
        );
        assert!(
            !result.contains("&amp;"),
            "Should not contain HTML entity for ampersand"
        );
    }
}
