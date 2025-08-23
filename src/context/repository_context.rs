use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

use super::FileSystemOperations;

/// Provides context about a repository including auto-detection of tech stack and project name
#[async_trait]
pub trait RepositoryContext: Send + Sync {
    /// Detects the technology stack based on repository files
    async fn detect_tech_stack(&self, repo_path: &Path) -> Result<String>;

    /// Detects the project name from the repository path
    async fn detect_project_name(&self, repo_path: &Path) -> Result<String>;
}

/// Default implementation of RepositoryContext
pub struct DefaultRepositoryContext {
    file_system: Arc<dyn FileSystemOperations>,
}

impl DefaultRepositoryContext {
    /// Creates a new DefaultRepositoryContext
    pub fn new(file_system: Arc<dyn FileSystemOperations>) -> Self {
        Self { file_system }
    }

    /// Checks if a file exists in the repository
    async fn file_exists(&self, repo_path: &Path, file_name: &str) -> bool {
        let file_path = repo_path.join(file_name);
        self.file_system.exists(&file_path).await.unwrap_or(false)
    }

    /// Cleans a project name to be suitable for Docker tags
    fn clean_project_name(name: &str) -> String {
        // Remove special characters and convert to lowercase
        let cleaned: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .to_lowercase();

        // Collapse consecutive dashes into a single dash
        let mut result = String::new();
        let mut prev_dash = false;
        for c in cleaned.chars() {
            if c == '-' {
                if !prev_dash {
                    result.push(c);
                }
                prev_dash = true;
            } else {
                result.push(c);
                prev_dash = false;
            }
        }

        // Trim dashes from both ends
        result.trim_matches('-').to_string()
    }
}

#[async_trait]
impl RepositoryContext for DefaultRepositoryContext {
    async fn detect_tech_stack(&self, repo_path: &Path) -> Result<String> {
        // Check for language-specific files in priority order
        let tech_stack = if self.file_exists(repo_path, "Cargo.toml").await {
            "rust"
        } else if self.file_exists(repo_path, "pyproject.toml").await
            || self.file_exists(repo_path, "requirements.txt").await
            || self.file_exists(repo_path, "setup.py").await
        {
            "python"
        } else if self.file_exists(repo_path, "package.json").await {
            "node"
        } else if self.file_exists(repo_path, "go.mod").await {
            "go"
        } else if self.file_exists(repo_path, "pom.xml").await
            || self.file_exists(repo_path, "build.gradle").await
            || self.file_exists(repo_path, "build.gradle.kts").await
        {
            "java"
        } else if self.file_exists(repo_path, "rockspec").await
            || self.file_exists(repo_path, ".luacheckrc").await
            || self.file_exists(repo_path, "init.lua").await
        {
            "lua"
        } else {
            "default"
        };

        Ok(tech_stack.to_string())
    }

    async fn detect_project_name(&self, repo_path: &Path) -> Result<String> {
        // Extract the directory name from the repository path
        let project_name = repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(Self::clean_project_name)
            .unwrap_or_else(|| "default".to_string());

        // Ensure the name is not empty after cleaning
        let project_name = if project_name.is_empty() {
            "default".to_string()
        } else {
            project_name
        };

        Ok(project_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock implementation for testing
    pub struct MockRepositoryContext {
        tech_stack: String,
        project_name: String,
    }

    impl MockRepositoryContext {
        /// Creates a new MockRepositoryContext with successful results
        pub fn new(tech_stack: String, project_name: String) -> Self {
            Self {
                tech_stack,
                project_name,
            }
        }
    }

    #[async_trait]
    impl RepositoryContext for MockRepositoryContext {
        async fn detect_tech_stack(&self, _repo_path: &Path) -> Result<String> {
            Ok(self.tech_stack.clone())
        }

        async fn detect_project_name(&self, _repo_path: &Path) -> Result<String> {
            Ok(self.project_name.clone())
        }
    }

    use crate::context::file_system::DefaultFileSystem;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_detect_rust_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create Cargo.toml file
        fs.write_file(&temp_dir.path().join("Cargo.toml"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "rust");
    }

    #[tokio::test]
    async fn test_detect_python_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create pyproject.toml file
        fs.write_file(&temp_dir.path().join("pyproject.toml"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "python");
    }

    #[tokio::test]
    async fn test_detect_python_tech_stack_requirements() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create requirements.txt file
        fs.write_file(&temp_dir.path().join("requirements.txt"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "python");
    }

    #[tokio::test]
    async fn test_detect_node_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create package.json file
        fs.write_file(&temp_dir.path().join("package.json"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "node");
    }

    #[tokio::test]
    async fn test_detect_go_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create go.mod file
        fs.write_file(&temp_dir.path().join("go.mod"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "go");
    }

    #[tokio::test]
    async fn test_detect_java_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create pom.xml file
        fs.write_file(&temp_dir.path().join("pom.xml"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "java");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_rockspec() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create rockspec file
        fs.write_file(&temp_dir.path().join("rockspec"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_luacheckrc() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create .luacheckrc file
        fs.write_file(&temp_dir.path().join(".luacheckrc"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_init() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // Create init.lua file
        fs.write_file(&temp_dir.path().join("init.lua"), "")
            .await
            .unwrap();

        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_default_tech_stack() {
        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);

        // No special files created
        let repo_context = DefaultRepositoryContext::new(fs);
        let result = repo_context
            .detect_tech_stack(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(result, "default");
    }

    #[tokio::test]
    async fn test_detect_project_name() {
        let fs = Arc::new(DefaultFileSystem);
        let repo_context = DefaultRepositoryContext::new(fs);

        // Create a temp dir with a specific name to test
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("my-awesome-project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = repo_context
            .detect_project_name(&project_dir)
            .await
            .unwrap();
        assert_eq!(result, "my-awesome-project");
    }

    #[tokio::test]
    async fn test_clean_project_name() {
        let fs = Arc::new(DefaultFileSystem);
        let repo_context = DefaultRepositoryContext::new(fs);

        // Create a temp dir with special characters in the name
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("My_Awesome Project!");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = repo_context
            .detect_project_name(&project_dir)
            .await
            .unwrap();
        assert_eq!(result, "my-awesome-project");
    }

    #[tokio::test]
    async fn test_project_name_with_special_chars() {
        let fs = Arc::new(DefaultFileSystem);
        let repo_context = DefaultRepositoryContext::new(fs);

        // Create a temp dir with special characters
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("test@#$%project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = repo_context
            .detect_project_name(&project_dir)
            .await
            .unwrap();
        assert_eq!(result, "test-project");
    }

    #[tokio::test]
    async fn test_project_name_fallback() {
        let fs = Arc::new(DefaultFileSystem);
        let repo_context = DefaultRepositoryContext::new(fs);

        let result = repo_context
            .detect_project_name(Path::new("/"))
            .await
            .unwrap();
        assert_eq!(result, "default");
    }

    #[tokio::test]
    async fn test_mock_repository_context() {
        let mock =
            MockRepositoryContext::new("custom-stack".to_string(), "custom-project".to_string());

        assert_eq!(
            mock.detect_tech_stack(Path::new("/any")).await.unwrap(),
            "custom-stack"
        );
        assert_eq!(
            mock.detect_project_name(Path::new("/any")).await.unwrap(),
            "custom-project"
        );
    }
}
