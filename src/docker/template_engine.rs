use anyhow::{Context, Result};
use handlebars::Handlebars;
use std::collections::HashMap;

use crate::assets::AssetManager;
use crate::docker::composer::{InlineLayerOverrides, LayerSource, LayerSources};

/// Template engine for rendering Dockerfiles using Handlebars templates
pub struct DockerTemplateEngine<'a> {
    handlebars: Handlebars<'a>,
    asset_manager: &'a dyn AssetManager,
    overrides: Option<&'a InlineLayerOverrides>,
}

impl<'a> DockerTemplateEngine<'a> {
    /// Creates a new Docker template engine
    pub fn new(
        asset_manager: &'a dyn AssetManager,
        overrides: Option<&'a InlineLayerOverrides>,
    ) -> Self {
        Self {
            handlebars: Handlebars::new(),
            asset_manager,
            overrides,
        }
    }

    /// Renders a Dockerfile by replacing template placeholders with layer contents.
    ///
    /// Returns the rendered Dockerfile content and the sources of each layer.
    pub fn render_dockerfile(
        &self,
        base_template: &str,
        stack: Option<&str>,
        agent: Option<&str>,
        project: Option<&str>,
    ) -> Result<(String, LayerSources)> {
        let mut context = HashMap::new();
        let mut layer_sources = LayerSources::default();

        // Get layer contents, checking overrides first
        let (stack_content, stack_source) = self.get_layer_content("stack", stack)?;
        context.insert("STACK", stack_content);
        layer_sources.stack = stack_source;

        let (agent_content, agent_source) = self.get_layer_content("agent", agent)?;
        context.insert("AGENT", agent_content);
        layer_sources.agent = agent_source;

        let (project_content, project_source) = self.get_layer_content("project", project)?;
        context.insert("PROJECT", project_content);
        layer_sources.project = project_source;

        let rendered = self
            .handlebars
            .render_template(base_template, &context)
            .context("Failed to render Dockerfile template")?;

        Ok((rendered, layer_sources))
    }

    /// Gets the content of a layer snippet, checking inline overrides first.
    ///
    /// Returns the content string and the source it came from.
    fn get_layer_content(
        &self,
        layer_type: &str,
        layer_name: Option<&str>,
    ) -> Result<(String, LayerSource)> {
        // Check inline overrides first
        if let Some(overrides) = self.overrides {
            let override_content = match layer_type {
                "stack" => overrides.stack_setup.as_deref(),
                "agent" => overrides.agent_setup.as_deref(),
                "project" => overrides.project_setup.as_deref(),
                _ => None,
            };
            if let Some(content) = override_content {
                return Ok((content.to_string(), LayerSource::Config));
            }
        }

        // Fall back to asset manager
        match layer_name {
            Some(name) => {
                let path = format!("{}/{}", layer_type, name);
                match self.asset_manager.get_dockerfile(&path) {
                    Ok(content) => {
                        let s = String::from_utf8(content)
                            .context("Failed to decode layer content as UTF-8")?;
                        Ok((s, LayerSource::AssetManager))
                    }
                    Err(_) => Ok((String::new(), LayerSource::Empty)),
                }
            }
            None => Ok((String::new(), LayerSource::Empty)),
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
        let engine = DockerTemplateEngine::new(&asset_manager, None);

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
        let (result, sources) = engine
            .render_dockerfile(base_template, Some("rust"), Some("claude"), Some("default"))
            .unwrap();

        assert!(result.contains("FROM ubuntu:24.04"));
        assert!(result.contains("# Stack layer"));
        assert!(result.contains("# Agent layer"));
        assert!(result.contains("# Project layer"));
        assert_eq!(sources.stack, LayerSource::AssetManager);
        assert_eq!(sources.agent, LayerSource::AssetManager);
    }

    #[test]
    fn test_render_dockerfile_with_missing_layers() {
        let asset_manager = EmbeddedAssetManager::new();
        let engine = DockerTemplateEngine::new(&asset_manager, None);

        let base_template = r#"FROM ubuntu:24.04
{{{STACK}}}
{{{AGENT}}}
{{{PROJECT}}}
"#;

        // Use non-existent layer names
        let (result, sources) = engine
            .render_dockerfile(
                base_template,
                Some("nonexistent"),
                Some("missing"),
                Some("notfound"),
            )
            .unwrap();

        // Should still render with empty content for missing layers
        assert_eq!(result, "FROM ubuntu:24.04\n\n\n\n");
        assert_eq!(sources.stack, LayerSource::Empty);
        assert_eq!(sources.agent, LayerSource::Empty);
        assert_eq!(sources.project, LayerSource::Empty);
    }

    #[test]
    fn test_render_dockerfile_with_no_layers() {
        let asset_manager = EmbeddedAssetManager::new();
        let engine = DockerTemplateEngine::new(&asset_manager, None);

        let base_template = r#"FROM ubuntu:24.04
{{{STACK}}}
{{{AGENT}}}
{{{PROJECT}}}
CMD ["/bin/bash"]"#;

        let (result, _) = engine
            .render_dockerfile(base_template, None, None, None)
            .unwrap();

        assert_eq!(result, "FROM ubuntu:24.04\n\n\n\nCMD [\"/bin/bash\"]");
    }

    #[test]
    fn test_inline_override_stack_layer() {
        let asset_manager = EmbeddedAssetManager::new();
        let overrides = InlineLayerOverrides {
            stack_setup: Some("RUN apt-get install -y custom-stack-tool".to_string()),
            ..Default::default()
        };
        let engine = DockerTemplateEngine::new(&asset_manager, Some(&overrides));

        let base_template = "FROM ubuntu:24.04\n{{{STACK}}}\n{{{AGENT}}}\n{{{PROJECT}}}";

        let (result, sources) = engine
            .render_dockerfile(base_template, Some("rust"), Some("claude"), Some("default"))
            .unwrap();

        assert!(result.contains("RUN apt-get install -y custom-stack-tool"));
        assert_eq!(sources.stack, LayerSource::Config);
        assert_eq!(sources.agent, LayerSource::AssetManager);
    }

    #[test]
    fn test_inline_override_agent_layer() {
        let asset_manager = EmbeddedAssetManager::new();
        let overrides = InlineLayerOverrides {
            agent_setup: Some("RUN npm install -g custom-agent".to_string()),
            ..Default::default()
        };
        let engine = DockerTemplateEngine::new(&asset_manager, Some(&overrides));

        let base_template = "FROM ubuntu:24.04\n{{{STACK}}}\n{{{AGENT}}}\n{{{PROJECT}}}";

        let (result, sources) = engine
            .render_dockerfile(
                base_template,
                Some("default"),
                Some("claude"),
                Some("default"),
            )
            .unwrap();

        assert!(result.contains("RUN npm install -g custom-agent"));
        assert_eq!(sources.agent, LayerSource::Config);
        assert_eq!(sources.stack, LayerSource::AssetManager);
    }

    #[test]
    fn test_inline_override_project_layer() {
        let asset_manager = EmbeddedAssetManager::new();
        let overrides = InlineLayerOverrides {
            project_setup: Some("RUN pip install project-dep".to_string()),
            ..Default::default()
        };
        let engine = DockerTemplateEngine::new(&asset_manager, Some(&overrides));

        let base_template = "FROM ubuntu:24.04\n{{{STACK}}}\n{{{AGENT}}}\n{{{PROJECT}}}";

        let (result, sources) = engine
            .render_dockerfile(
                base_template,
                Some("default"),
                Some("claude"),
                Some("default"),
            )
            .unwrap();

        assert!(result.contains("RUN pip install project-dep"));
        assert_eq!(sources.project, LayerSource::Config);
    }

    #[test]
    fn test_inline_override_with_no_fallback() {
        let asset_manager = EmbeddedAssetManager::new();
        let overrides = InlineLayerOverrides {
            stack_setup: Some("RUN install-custom-stack".to_string()),
            ..Default::default()
        };
        let engine = DockerTemplateEngine::new(&asset_manager, Some(&overrides));

        let base_template = "FROM ubuntu:24.04\n{{{STACK}}}\n{{{AGENT}}}\n{{{PROJECT}}}";

        // Use a stack name that doesn't exist in embedded assets -- override should still work
        let (result, sources) = engine
            .render_dockerfile(
                base_template,
                Some("nonexistent-stack"),
                Some("claude"),
                Some("default"),
            )
            .unwrap();

        assert!(result.contains("RUN install-custom-stack"));
        assert_eq!(sources.stack, LayerSource::Config);
    }

    #[test]
    fn test_no_override_falls_back_to_asset_manager() {
        let asset_manager = EmbeddedAssetManager::new();
        let overrides = InlineLayerOverrides::default(); // no overrides set
        let engine = DockerTemplateEngine::new(&asset_manager, Some(&overrides));

        let base_template = "FROM ubuntu:24.04\n{{{STACK}}}\n{{{AGENT}}}\n{{{PROJECT}}}";

        let (_, sources) = engine
            .render_dockerfile(base_template, Some("rust"), Some("claude"), Some("default"))
            .unwrap();

        // All should come from embedded assets
        assert_eq!(sources.stack, LayerSource::AssetManager);
        assert_eq!(sources.agent, LayerSource::AssetManager);
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
        let engine = DockerTemplateEngine::new(&asset_manager, None);

        let base_template = r#"FROM ubuntu:24.04
# Stack layer
{{{STACK}}}
# End of Stack layer"#;

        let (result, _) = engine
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
