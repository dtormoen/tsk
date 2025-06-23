use std::fs;
use tempfile::TempDir;
use tsk::assets::{filesystem::FileSystemAssetManager, layered::LayeredAssetManager, AssetManager};
use tsk::context::AppContext;
use tsk::storage::XdgDirectories;

#[test]
fn test_layered_template_resolution_priority() {
    // Create temporary directories
    let temp_dir = TempDir::new().unwrap();

    // Create project directory with .tsk/templates
    let project_dir = temp_dir.path().join("project");
    let project_templates = project_dir.join(".tsk").join("templates");
    fs::create_dir_all(&project_templates).unwrap();
    fs::write(
        project_templates.join("feature.md"),
        "Project feature template",
    )
    .unwrap();
    fs::write(project_templates.join("fix.md"), "Project fix template").unwrap();

    // Create user config directory
    let config_dir = temp_dir.path().join("config");
    let user_templates = config_dir.join("templates");
    fs::create_dir_all(&user_templates).unwrap();
    fs::write(user_templates.join("feature.md"), "User feature template").unwrap();
    fs::write(user_templates.join("doc.md"), "User doc template").unwrap();

    // Create XDG directories
    let xdg_dirs = XdgDirectories::new_with_paths(
        temp_dir.path().join("data"),
        temp_dir.path().join("runtime"),
        config_dir,
        temp_dir.path().join("cache"),
    );

    // Create layered asset manager
    let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

    // Test priority: project > user > built-in
    assert_eq!(
        manager.get_template("feature").unwrap(),
        "Project feature template"
    );
    assert_eq!(manager.get_template("fix").unwrap(), "Project fix template");
    assert_eq!(manager.get_template("doc").unwrap(), "User doc template");

    // Test built-in fallback (refactor template should be built-in only)
    assert!(manager
        .get_template("refactor")
        .unwrap()
        .contains("{{DESCRIPTION}}"));

    // Test listing all templates
    let all_templates = manager.list_templates();
    assert!(all_templates.contains(&"feature".to_string()));
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
    assert!(manager.get_template("feature").is_err());

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
async fn test_app_context_uses_layered_asset_manager() {
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

    // Change to repo directory
    std::env::set_current_dir(&repo_dir).unwrap();

    // Build AppContext
    let app_context = AppContext::builder().build();

    // Verify it can access the custom template
    let template = app_context.asset_manager().get_template("custom");
    assert!(template.is_ok());
    assert_eq!(template.unwrap(), "Custom project template");

    // Verify it still has access to built-in templates
    assert!(app_context.asset_manager().get_template("feature").is_ok());
}

#[test]
fn test_template_listing_deduplication() {
    let temp_dir = TempDir::new().unwrap();

    // Create project directory
    let project_dir = temp_dir.path().join("project");
    let project_templates = project_dir.join(".tsk").join("templates");
    fs::create_dir_all(&project_templates).unwrap();
    fs::write(project_templates.join("feature.md"), "Project feature").unwrap();

    // Create user config directory
    let config_dir = temp_dir.path().join("config");
    let user_templates = config_dir.join("templates");
    fs::create_dir_all(&user_templates).unwrap();
    fs::write(user_templates.join("feature.md"), "User feature").unwrap();
    fs::write(user_templates.join("custom.md"), "User custom").unwrap();

    let xdg_dirs = XdgDirectories::new_with_paths(
        temp_dir.path().join("data"),
        temp_dir.path().join("runtime"),
        config_dir,
        temp_dir.path().join("cache"),
    );

    let manager = LayeredAssetManager::new_with_standard_layers(Some(&project_dir), &xdg_dirs);

    let templates = manager.list_templates();

    // Should only have one "feature" entry (deduplicated)
    let feature_count = templates.iter().filter(|t| t == &"feature").count();
    assert_eq!(feature_count, 1);

    // Should have custom template
    assert!(templates.contains(&"custom".to_string()));
}
