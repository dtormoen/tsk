use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::assets::{
    AssetManager, embedded::EmbeddedAssetManager, filesystem::FileSystemAssetManager,
};
use crate::context::tsk_config::TskConfig;

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
    pub fn new_with_standard_layers(project_root: Option<&Path>, tsk_config: &TskConfig) -> Self {
        let mut layers: Vec<Arc<dyn AssetManager>> = Vec::new();

        // Project layer (highest priority)
        if let Some(root) = project_root {
            let project_tsk_dir = root.join(".tsk");
            if project_tsk_dir.exists() {
                layers.push(Arc::new(FileSystemAssetManager::new(project_tsk_dir)));
            }
        }

        // User layer - use the entire config directory to support both templates and dockerfiles
        let user_config_dir = tsk_config.config_dir();
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
    use crate::assets::filesystem::FileSystemAssetManager;
    use crate::context::AppContext;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Helper to create templates in a directory
    fn create_templates(base_path: &Path, templates: &[(&str, &str)]) {
        let templates_dir = base_path.join("templates");
        fs::create_dir_all(&templates_dir).unwrap();

        for (name, content) in templates {
            fs::write(templates_dir.join(format!("{}.md", name)), content).unwrap();
        }
    }

    /// Helper to create dockerfiles in a directory
    fn create_dockerfiles(base_path: &Path, dockerfiles: &[(&str, &str)]) {
        for (dockerfile_path, content) in dockerfiles {
            let full_path = base_path.join("dockerfiles").join(dockerfile_path);
            fs::create_dir_all(&full_path).unwrap();
            fs::write(full_path.join("Dockerfile"), content).unwrap();
        }
    }

    #[test]
    fn test_layered_get_template_priority() {
        let temp_dir = TempDir::new().unwrap();

        // Create layers with templates
        let layer1_dir = temp_dir.path().join("layer1");
        let layer2_dir = temp_dir.path().join("layer2");

        create_templates(&layer1_dir, &[("feature", "Layer 1 content")]);
        create_templates(
            &layer2_dir,
            &[
                ("feature", "Layer 2 content"),
                ("fix", "Layer 2 fix content"),
            ],
        );

        let manager = LayeredAssetManager::new(vec![
            Arc::new(FileSystemAssetManager::new(layer1_dir)),
            Arc::new(FileSystemAssetManager::new(layer2_dir)),
        ]);

        // Should get from first layer (higher priority)
        assert_eq!(manager.get_template("feature").unwrap(), "Layer 1 content");
        // Should get from second layer since first doesn't have it
        assert_eq!(manager.get_template("fix").unwrap(), "Layer 2 fix content");
        // Should fail if not in any layer
        assert!(manager.get_template("nonexistent").is_err());
    }

    #[test]
    fn test_layered_list_templates_aggregation() {
        let temp_dir = TempDir::new().unwrap();

        let layer1_dir = temp_dir.path().join("layer1");
        let layer2_dir = temp_dir.path().join("layer2");

        create_templates(
            &layer1_dir,
            &[("feature", "Layer 1 feature"), ("fix", "Layer 1 fix")],
        );
        create_templates(
            &layer2_dir,
            &[("fix", "Layer 2 fix"), ("doc", "Layer 2 doc")],
        );

        let manager = LayeredAssetManager::new(vec![
            Arc::new(FileSystemAssetManager::new(layer1_dir)),
            Arc::new(FileSystemAssetManager::new(layer2_dir)),
        ]);

        // Should contain all unique templates, sorted alphabetically
        assert_eq!(manager.list_templates(), vec!["doc", "feature", "fix"]);
    }

    #[test]
    fn test_standard_layers_with_project() {
        let app_context = AppContext::builder().build();
        let temp_dir = TempDir::new().unwrap();

        // Create project templates
        let project_tsk = temp_dir.path().join(".tsk");
        create_templates(&project_tsk, &[("feature", "Project template")]);

        let manager = LayeredAssetManager::new_with_standard_layers(
            Some(temp_dir.path()),
            &app_context.tsk_config(),
        );

        // Should get project template
        assert_eq!(manager.get_template("feature").unwrap(), "Project template");
    }

    #[test]
    fn test_layered_template_resolution_priority() {
        use crate::context::tsk_config::{TskConfig, XdgConfig};

        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let config_dir = temp_dir.path().join("config");

        // Create project templates
        let project_tsk = project_dir.join(".tsk");
        create_templates(
            &project_tsk,
            &[
                ("feat", "Project feat template"),
                ("fix", "Project fix template"),
            ],
        );

        // Create user templates
        let user_tsk = config_dir.join("tsk");
        create_templates(
            &user_tsk,
            &[("feat", "User feat template"), ("doc", "User doc template")],
        );

        // Create app context with custom config directory
        let xdg_config = XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build();
        let tsk_config = TskConfig::new(Some(xdg_config)).unwrap();

        let manager =
            LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &tsk_config);

        // Test priority: project > user > built-in
        assert_eq!(
            manager.get_template("feat").unwrap(),
            "Project feat template"
        );
        assert_eq!(manager.get_template("fix").unwrap(), "Project fix template");
        assert_eq!(manager.get_template("doc").unwrap(), "User doc template");

        // Test built-in fallback
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
        let manager = FileSystemAssetManager::new(temp_dir.path().join("nonexistent"));

        assert!(manager.get_template("feat").is_err());
        assert!(manager.list_templates().is_empty());
    }

    #[test]
    fn test_template_content_validation() {
        let temp_dir = TempDir::new().unwrap();
        create_templates(
            temp_dir.path(),
            &[(
                "valid",
                "# Feature Template\n\n{{DESCRIPTION}}\n\n## Best Practices\n- Test your code",
            )],
        );

        let manager = FileSystemAssetManager::new(temp_dir.path().to_path_buf());
        let template = manager.get_template("valid").unwrap();

        assert!(template.contains("{{DESCRIPTION}}"));
        assert!(template.contains("Best Practices"));
    }

    #[tokio::test]
    async fn test_app_context_with_layered_asset_manager() {
        let app_context = AppContext::builder().build();
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("repo");

        // Initialize git repo
        fs::create_dir_all(&repo_dir).unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_dir)
            .output()
            .expect("Failed to init git repo");

        // Create project-level template
        let project_tsk = repo_dir.join(".tsk");
        create_templates(&project_tsk, &[("custom", "Custom project template")]);

        let asset_manager = LayeredAssetManager::new_with_standard_layers(
            Some(&repo_dir),
            &app_context.tsk_config(),
        );

        // Verify it can access the custom template
        assert_eq!(
            asset_manager.get_template("custom").unwrap(),
            "Custom project template"
        );
        // Verify it still has access to built-in templates
        assert!(asset_manager.get_template("feat").is_ok());
    }

    #[test]
    fn test_template_listing_deduplication() {
        use crate::context::tsk_config::{TskConfig, XdgConfig};

        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let config_dir = temp_dir.path().join("config");

        // Create project and user templates with overlap
        create_templates(&project_dir.join(".tsk"), &[("feat", "Project feat")]);
        create_templates(
            &config_dir.join("tsk"),
            &[("feat", "User feat"), ("custom", "User custom")],
        );

        let xdg_config = XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build();
        let tsk_config = TskConfig::new(Some(xdg_config)).unwrap();

        let manager =
            LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &tsk_config);
        let templates = manager.list_templates();

        // Should only have one "feat" entry (deduplicated)
        assert_eq!(templates.iter().filter(|t| t == &"feat").count(), 1);
        // Should have custom template
        assert!(templates.contains(&"custom".to_string()));
    }

    #[test]
    fn test_dockerfile_layering() {
        use crate::context::tsk_config::{TskConfig, XdgConfig};

        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let config_dir = temp_dir.path().join("config");

        // Create project dockerfiles
        create_dockerfiles(
            &project_dir.join(".tsk"),
            &[("project/myapp", "FROM ubuntu:project\n")],
        );

        // Create user dockerfiles
        create_dockerfiles(
            &config_dir.join("tsk"),
            &[
                ("project/myapp", "FROM ubuntu:user\n"),
                ("project/otherapp", "FROM ubuntu:user-only\n"),
            ],
        );

        let xdg_config = XdgConfig::builder()
            .with_data_dir(temp_dir.path().join("data"))
            .with_runtime_dir(temp_dir.path().join("runtime"))
            .with_config_dir(config_dir)
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build();
        let tsk_config = TskConfig::new(Some(xdg_config)).unwrap();

        let manager =
            LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &tsk_config);

        // Test that project dockerfile takes precedence
        let myapp_content =
            String::from_utf8(manager.get_dockerfile("project/myapp").unwrap()).unwrap();
        assert!(myapp_content.contains("FROM ubuntu:project"));

        // Test that user-only dockerfile is accessible
        let otherapp_content =
            String::from_utf8(manager.get_dockerfile("project/otherapp").unwrap()).unwrap();
        assert!(otherapp_content.contains("FROM ubuntu:user-only"));

        // Test listing dockerfiles - should show both
        let dockerfiles = manager.list_dockerfiles();
        assert!(dockerfiles.contains(&"project/myapp".to_string()));
        assert!(dockerfiles.contains(&"project/otherapp".to_string()));
    }
}
