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
    /// 1. Project-level templates (.tsk/templates)
    /// 2. User-level templates (~/.config/tsk/templates)
    /// 3. Built-in templates (embedded)
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

        // User layer
        let user_templates_dir = xdg_dirs.config_dir().join("templates");
        if user_templates_dir.exists() {
            layers.push(Arc::new(FileSystemAssetManager::new(user_templates_dir)));
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

    struct MockAssetManager {
        templates: Vec<String>,
        template_content: String,
    }

    impl MockAssetManager {
        fn new(templates: Vec<String>, template_content: String) -> Self {
            Self {
                templates,
                template_content,
            }
        }
    }

    #[async_trait]
    impl AssetManager for MockAssetManager {
        fn get_template(&self, template_type: &str) -> Result<String> {
            if self.templates.contains(&template_type.to_string()) {
                Ok(self.template_content.clone())
            } else {
                Err(anyhow::anyhow!("Template not found"))
            }
        }

        fn get_dockerfile(&self, _dockerfile_name: &str) -> Result<Vec<u8>> {
            Err(anyhow::anyhow!("Not implemented"))
        }

        fn get_dockerfile_file(&self, _dockerfile_name: &str, _file_path: &str) -> Result<Vec<u8>> {
            Err(anyhow::anyhow!("Not implemented"))
        }

        fn list_templates(&self) -> Vec<String> {
            self.templates.clone()
        }

        fn list_dockerfiles(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn test_layered_get_template_priority() {
        let layer1 = Arc::new(MockAssetManager::new(
            vec!["feature".to_string()],
            "Layer 1 content".to_string(),
        ));
        let layer2 = Arc::new(MockAssetManager::new(
            vec!["feature".to_string(), "fix".to_string()],
            "Layer 2 content".to_string(),
        ));

        let manager = LayeredAssetManager::new(vec![layer1, layer2]);

        // Should get from first layer
        assert_eq!(manager.get_template("feature").unwrap(), "Layer 1 content");

        // Should get from second layer since first doesn't have it
        assert_eq!(manager.get_template("fix").unwrap(), "Layer 2 content");

        // Should fail if not in any layer
        assert!(manager.get_template("nonexistent").is_err());
    }

    #[test]
    fn test_layered_list_templates_aggregation() {
        let layer1 = Arc::new(MockAssetManager::new(
            vec!["feature".to_string(), "fix".to_string()],
            "content".to_string(),
        ));
        let layer2 = Arc::new(MockAssetManager::new(
            vec!["fix".to_string(), "doc".to_string()],
            "content".to_string(),
        ));

        let manager = LayeredAssetManager::new(vec![layer1, layer2]);
        let templates = manager.list_templates();

        assert_eq!(templates, vec!["doc", "feature", "fix"]);
    }

    #[test]
    fn test_standard_layers_with_project() {
        let temp_dir = TempDir::new().unwrap();

        // Create project templates directory
        let project_templates = temp_dir.path().join(".tsk").join("templates");
        fs::create_dir_all(&project_templates).unwrap();
        fs::write(project_templates.join("feature.md"), "Project template").unwrap();

        // Create mock XDG directories
        let config_dir = temp_dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();

        let xdg_config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            config_dir,
        );
        let xdg_dirs = XdgDirectories::new(Some(xdg_config)).unwrap();

        let manager =
            LayeredAssetManager::new_with_standard_layers(Some(temp_dir.path()), &xdg_dirs);

        // Should get project template
        assert_eq!(manager.get_template("feature").unwrap(), "Project template");
    }

    #[test]
    fn test_layered_template_resolution_priority() {
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
        let user_templates = config_dir.join("tsk").join("templates");
        fs::create_dir_all(&user_templates).unwrap();
        fs::write(user_templates.join("feat.md"), "User feat template").unwrap();
        fs::write(user_templates.join("doc.md"), "User doc template").unwrap();

        // Create XDG directories
        let xdg_config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            config_dir,
        );
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
            .args(&["init"])
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
        let user_templates = config_dir.join("tsk").join("templates");
        fs::create_dir_all(&user_templates).unwrap();
        fs::write(user_templates.join("feat.md"), "User feat").unwrap();
        fs::write(user_templates.join("custom.md"), "User custom").unwrap();

        let xdg_config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            config_dir,
        );
        let xdg_dirs = XdgDirectories::new(Some(xdg_config)).unwrap();

        let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

        let templates = manager.list_templates();

        // Should only have one "feat" entry (deduplicated)
        let feat_count = templates.iter().filter(|t| t == &"feat").count();
        assert_eq!(feat_count, 1);

        // Should have custom template
        assert!(templates.contains(&"custom".to_string()));
    }
}
