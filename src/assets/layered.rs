use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::assets::{
    AssetManager, embedded::EmbeddedAssetManager, filesystem::FileSystemAssetManager,
};
use crate::storage::xdg::XdgDirectories;

/// A layered implementation of AssetManager that checks multiple sources in priority order
pub struct LayeredAssetManager {
    layers: Vec<Arc<dyn AssetManager>>,
}

impl LayeredAssetManager {
    /// Creates a new LayeredAssetManager with the given layers in priority order
    pub fn new(layers: Vec<Arc<dyn AssetManager>>) -> Self {
        Self { layers }
    }

    /// Creates a LayeredAssetManager with standard layers:
    /// 1. Project-level assets (.tsk/)
    /// 2. User-level assets (~/.config/tsk/)
    /// 3. Built-in assets (embedded)
    pub fn new_with_standard_layers(
        project_root: Option<&Path>,
        xdg_dirs: &XdgDirectories,
    ) -> Self {
        let mut layers: Vec<Arc<dyn AssetManager>> = Vec::new();

        // Project layer (highest priority)
        if let Some(root) = project_root {
            let project_tsk_dir = root.join(".tsk");
            if project_tsk_dir.exists() {
                layers.push(Arc::new(FileSystemAssetManager::new(project_tsk_dir)));
            }
        }

        // User layer - use the entire config directory to support both templates and dockerfiles
        let user_config_dir = xdg_dirs.config_dir();
        if user_config_dir.exists() {
            layers.push(Arc::new(FileSystemAssetManager::new(
                user_config_dir.to_path_buf(),
            )));
        }

        // Built-in layer (lowest priority)
        layers.push(Arc::new(EmbeddedAssetManager));

        Self::new(layers)
    }
}

#[async_trait]
impl AssetManager for LayeredAssetManager {
    fn get_template(&self, template_type: &str) -> Result<String> {
        for layer in &self.layers {
            match layer.get_template(template_type) {
                Ok(template) => return Ok(template),
                Err(_) => continue,
            }
        }

        Err(anyhow::anyhow!(
            "Template '{}' not found in any layer",
            template_type
        ))
    }

    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>> {
        for layer in &self.layers {
            match layer.get_dockerfile(dockerfile_name) {
                Ok(content) => return Ok(content),
                Err(_) => continue,
            }
        }

        Err(anyhow::anyhow!(
            "Dockerfile '{}' not found",
            dockerfile_name
        ))
    }

    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>> {
        for layer in &self.layers {
            match layer.get_dockerfile_file(dockerfile_name, file_path) {
                Ok(content) => return Ok(content),
                Err(_) => continue,
            }
        }

        Err(anyhow::anyhow!(
            "Dockerfile file '{}/{}' not found",
            dockerfile_name,
            file_path
        ))
    }

    fn list_templates(&self) -> Vec<String> {
        let mut templates = HashSet::new();

        for layer in &self.layers {
            for template in layer.list_templates() {
                templates.insert(template);
            }
        }

        let mut result: Vec<String> = templates.into_iter().collect();
        result.sort();
        result
    }

    fn list_dockerfiles(&self) -> Vec<String> {
        let mut dockerfiles = HashSet::new();

        for layer in &self.layers {
            for dockerfile in layer.list_dockerfiles() {
                dockerfiles.insert(dockerfile);
            }
        }

        let mut result: Vec<String> = dockerfiles.into_iter().collect();
        result.sort();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{
        AssetManager, filesystem::FileSystemAssetManager, layered::LayeredAssetManager,
    };
    use crate::context::AppContext;
    use crate::storage::XdgDirectories;
    use std::fs;
    use tempfile::TempDir;

    /// Creates a temporary directory with templates for testing
    fn create_temp_templates_dir(templates: &[(&str, &str)]) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir_all(&templates_dir).unwrap();

        for (name, content) in templates {
            fs::write(templates_dir.join(format!("{}.md", name)), content).unwrap();
        }

        temp_dir
    }

    #[test]
    fn test_layered_get_template_priority() {
        // Create first layer with "feature" template
        let layer1_dir = create_temp_templates_dir(&[("feature", "Layer 1 content")]);

        // Create second layer with "feature" and "fix" templates
        let layer2_dir = create_temp_templates_dir(&[
            ("feature", "Layer 2 content"),
            ("fix", "Layer 2 fix content"),
        ]);

        let layer1 = Arc::new(FileSystemAssetManager::new(layer1_dir.path().to_path_buf()));
        let layer2 = Arc::new(FileSystemAssetManager::new(layer2_dir.path().to_path_buf()));

        let manager = LayeredAssetManager::new(vec![layer1, layer2]);

        // Should get from first layer (higher priority)
        assert_eq!(manager.get_template("feature").unwrap(), "Layer 1 content");

        // Should get from second layer since first doesn't have it
        assert_eq!(manager.get_template("fix").unwrap(), "Layer 2 fix content");

        // Should fail if not in any layer
        assert!(manager.get_template("nonexistent").is_err());
    }

    #[test]
    fn test_layered_list_templates_aggregation() {
        // Create first layer with "feature" and "fix" templates
        let layer1_dir =
            create_temp_templates_dir(&[("feature", "Layer 1 feature"), ("fix", "Layer 1 fix")]);

        // Create second layer with "fix" and "doc" templates
        let layer2_dir =
            create_temp_templates_dir(&[("fix", "Layer 2 fix"), ("doc", "Layer 2 doc")]);

        let layer1 = Arc::new(FileSystemAssetManager::new(layer1_dir.path().to_path_buf()));
        let layer2 = Arc::new(FileSystemAssetManager::new(layer2_dir.path().to_path_buf()));

        let manager = LayeredAssetManager::new(vec![layer1, layer2]);
        let templates = manager.list_templates();

        // Should contain all unique templates, sorted alphabetically
        assert_eq!(templates, vec!["doc", "feature", "fix"]);
    }

    #[test]
    fn test_standard_layers_with_project() {
        use crate::context::AppContext;

        let app_context = AppContext::builder().build();
        let xdg_dirs = app_context.xdg_directories();

        // Create project templates directory in the test temp directory
        let temp_dir = TempDir::new().unwrap();
        let project_templates = temp_dir.path().join(".tsk").join("templates");
        fs::create_dir_all(&project_templates).unwrap();
        fs::write(project_templates.join("feature.md"), "Project template").unwrap();

        let manager =
            LayeredAssetManager::new_with_standard_layers(Some(temp_dir.path()), &xdg_dirs);

        // Should get project template
        assert_eq!(manager.get_template("feature").unwrap(), "Project template");
    }

    #[test]
    fn test_layered_template_resolution_priority() {
        use crate::storage::XdgConfig;

        // Create temporary directories
        let temp_dir = TempDir::new().unwrap();

        // Create project directory with .tsk/templates
        let project_dir = temp_dir.path().join("project");
        let project_templates = project_dir.join(".tsk").join("templates");
        fs::create_dir_all(&project_templates).unwrap();
        fs::write(project_templates.join("feat.md"), "Project feat template").unwrap();
        fs::write(project_templates.join("fix.md"), "Project fix template").unwrap();

        // Create user config directory
        let config_dir = temp_dir.path().join("config");
        let user_tsk_dir = config_dir.join("tsk");
        let user_templates = user_tsk_dir.join("templates");
        fs::create_dir_all(&user_templates).unwrap();
        fs::write(user_templates.join("feat.md"), "User feat template").unwrap();
        fs::write(user_templates.join("doc.md"), "User doc template").unwrap();

        // Create XDG config with the custom config directory to use user templates
        let xdg_config = XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .build();
        let xdg_dirs = XdgDirectories::new(Some(xdg_config)).unwrap();

        // Create layered asset manager
        let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

        // Test priority: project > user > built-in
        assert_eq!(
            manager.get_template("feat").unwrap(),
            "Project feat template"
        );
        assert_eq!(manager.get_template("fix").unwrap(), "Project fix template");
        assert_eq!(manager.get_template("doc").unwrap(), "User doc template");

        // Test built-in fallback (refactor template should be built-in only)
        assert!(
            manager
                .get_template("refactor")
                .unwrap()
                .contains("{{DESCRIPTION}}")
        );

        // Test listing all templates
        let all_templates = manager.list_templates();
        assert!(all_templates.contains(&"feat".to_string()));
        assert!(all_templates.contains(&"fix".to_string()));
        assert!(all_templates.contains(&"doc".to_string()));
        assert!(all_templates.contains(&"refactor".to_string()));
    }

    #[test]
    fn test_filesystem_asset_manager_with_missing_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent_dir = temp_dir.path().join("nonexistent");

        let manager = FileSystemAssetManager::new(nonexistent_dir);

        // Should return error for missing templates
        assert!(manager.get_template("feat").is_err());

        // Should return empty list
        assert!(manager.list_templates().is_empty());
    }

    #[test]
    fn test_template_content_validation() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir_all(&templates_dir).unwrap();

        // Create a valid template with placeholder
        let valid_content =
            "# Feature Template\n\n{{DESCRIPTION}}\n\n## Best Practices\n- Test your code";
        fs::write(templates_dir.join("valid.md"), valid_content).unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let template = manager.get_template("valid").unwrap();

        // Verify template contains expected placeholder
        assert!(template.contains("{{DESCRIPTION}}"));
        assert!(template.contains("Best Practices"));
    }

    #[tokio::test]
    async fn test_app_context_with_layered_asset_manager() {
        // Create a temporary git repository
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&repo_dir).unwrap();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_dir)
            .output()
            .expect("Failed to init git repo");

        // Create project-level template
        let project_templates = repo_dir.join(".tsk").join("templates");
        fs::create_dir_all(&project_templates).unwrap();
        fs::write(
            project_templates.join("custom.md"),
            "Custom project template",
        )
        .unwrap();

        // Build AppContext
        let app_context = AppContext::builder().build();

        // Create asset manager on-demand
        let asset_manager = LayeredAssetManager::new_with_standard_layers(
            Some(&repo_dir),
            &app_context.xdg_directories(),
        );

        // Verify it can access the custom template
        let template = asset_manager.get_template("custom");
        assert!(template.is_ok());
        assert_eq!(template.unwrap(), "Custom project template");

        // Verify it still has access to built-in templates
        assert!(asset_manager.get_template("feat").is_ok());
    }

    #[test]
    fn test_template_listing_deduplication() {
        let temp_dir = TempDir::new().unwrap();

        // Create project directory
        let project_dir = temp_dir.path().join("project");
        let project_templates = project_dir.join(".tsk").join("templates");
        fs::create_dir_all(&project_templates).unwrap();
        fs::write(project_templates.join("feat.md"), "Project feat").unwrap();

        // Create user config directory
        let config_dir = temp_dir.path().join("config");
        let user_tsk_dir = config_dir.join("tsk");
        let user_templates = user_tsk_dir.join("templates");
        fs::create_dir_all(&user_templates).unwrap();
        fs::write(user_templates.join("feat.md"), "User feat").unwrap();
        fs::write(user_templates.join("custom.md"), "User custom").unwrap();

        let xdg_config = crate::storage::XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .build();
        let xdg_dirs = XdgDirectories::new(Some(xdg_config)).unwrap();

        let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

        let templates = manager.list_templates();

        // Should only have one "feat" entry (deduplicated)
        let feat_count = templates.iter().filter(|t| t == &"feat").count();
        assert_eq!(feat_count, 1);

        // Should have custom template
        assert!(templates.contains(&"custom".to_string()));
    }

    #[test]
    fn test_dockerfile_layering() {
        let temp_dir = TempDir::new().unwrap();

        // Create project directory with project-specific dockerfiles
        let project_dir = temp_dir.path().join("project");
        let project_dockerfiles = project_dir
            .join(".tsk")
            .join("dockerfiles")
            .join("project")
            .join("myapp");
        fs::create_dir_all(&project_dockerfiles).unwrap();
        fs::write(
            project_dockerfiles.join("Dockerfile"),
            "FROM ubuntu:project\n",
        )
        .unwrap();

        // Create user config directory with project-specific dockerfiles
        let config_dir = temp_dir.path().join("config");
        let user_tsk_dir = config_dir.join("tsk");
        let user_dockerfiles = user_tsk_dir
            .join("dockerfiles")
            .join("project")
            .join("myapp");
        fs::create_dir_all(&user_dockerfiles).unwrap();
        fs::write(user_dockerfiles.join("Dockerfile"), "FROM ubuntu:user\n").unwrap();

        // Create another dockerfile in user config that doesn't exist in project
        let user_only_dockerfiles = user_tsk_dir
            .join("dockerfiles")
            .join("project")
            .join("otherapp");
        fs::create_dir_all(&user_only_dockerfiles).unwrap();
        fs::write(
            user_only_dockerfiles.join("Dockerfile"),
            "FROM ubuntu:user-only\n",
        )
        .unwrap();

        let xdg_config = crate::storage::XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .build();
        let xdg_dirs = XdgDirectories::new(Some(xdg_config)).unwrap();

        let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

        // Test that project dockerfile takes precedence
        let myapp_dockerfile = manager.get_dockerfile("project/myapp").unwrap();
        let content = String::from_utf8(myapp_dockerfile).unwrap();
        assert!(content.contains("FROM ubuntu:project"));

        // Test that user-only dockerfile is accessible
        let otherapp_dockerfile = manager.get_dockerfile("project/otherapp").unwrap();
        let content = String::from_utf8(otherapp_dockerfile).unwrap();
        assert!(content.contains("FROM ubuntu:user-only"));

        // Test listing dockerfiles - should show both
        let dockerfiles = manager.list_dockerfiles();
        assert!(dockerfiles.contains(&"project/myapp".to_string()));
        assert!(dockerfiles.contains(&"project/otherapp".to_string()));
    }
}
