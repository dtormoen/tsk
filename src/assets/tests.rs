//! Tests for the asset management system

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::assets::embedded::EmbeddedAssetManager;

    #[test]
    fn test_embedded_asset_manager_creation() {
        let _manager = EmbeddedAssetManager::new();
    }

    #[test]
    fn test_get_template_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a known template
        let result = manager.get_template("feat");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Feature"));
        assert!(content.contains("{{DESCRIPTION}}"));
    }

    #[test]
    fn test_get_template_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent template
        let result = manager.get_template("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_templates() {
        let manager = EmbeddedAssetManager::new();

        let templates = manager.list_templates();
        assert!(!templates.is_empty());

        // Check that known templates are included
        assert!(templates.contains(&"feat".to_string()));
        assert!(templates.contains(&"fix".to_string()));
        assert!(templates.contains(&"doc".to_string()));
        assert!(templates.contains(&"plan".to_string()));
        assert!(templates.contains(&"refactor".to_string()));
    }

    #[test]
    fn test_get_dockerfile_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting the tsk-base Dockerfile
        let result = manager.get_dockerfile("tsk-base");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        // Convert to string to check content
        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("FROM"));
    }

    #[test]
    fn test_get_dockerfile_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent dockerfile
        let result = manager.get_dockerfile("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_dockerfiles() {
        let manager = EmbeddedAssetManager::new();

        let dockerfiles = manager.list_dockerfiles();
        assert!(!dockerfiles.is_empty());

        // Check that known dockerfiles are included
        assert!(dockerfiles.contains(&"tsk-base".to_string()));
        assert!(dockerfiles.contains(&"tsk-proxy".to_string()));
    }

    #[test]
    fn test_get_dockerfile_file_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting squid.conf from tsk-proxy
        let result = manager.get_dockerfile_file("tsk-proxy", "squid.conf");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        // Convert to string to check content
        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("http_access"));
    }

    #[test]
    fn test_get_dockerfile_file_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent file
        let result = manager.get_dockerfile_file("tsk-base", "nonexistent.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_asset_extraction() {
        use crate::assets::utils::extract_dockerfile_to_temp;

        let manager = EmbeddedAssetManager::new();

        // Test extracting tsk-base dockerfile
        let result = extract_dockerfile_to_temp(&manager, "tsk-base");
        assert!(result.is_ok());

        let temp_dir = result.unwrap();
        assert!(temp_dir.exists());
        assert!(temp_dir.join("Dockerfile").exists());

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_asset_extraction_with_additional_files() {
        use crate::assets::utils::extract_dockerfile_to_temp;

        let manager = EmbeddedAssetManager::new();

        // Test extracting tsk-proxy dockerfile (which has squid.conf)
        let result = extract_dockerfile_to_temp(&manager, "tsk-proxy");
        assert!(result.is_ok());

        let temp_dir = result.unwrap();
        assert!(temp_dir.exists());
        assert!(temp_dir.join("Dockerfile").exists());
        assert!(temp_dir.join("squid.conf").exists());

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
