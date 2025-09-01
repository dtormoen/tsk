use anyhow::Result;
use std::path::Path;

/// Detects the stack based on repository files
///
/// Checks for language-specific marker files in priority order:
/// - Rust: `Cargo.toml` → "rust"
/// - Python: `pyproject.toml`, `requirements.txt`, `setup.py` → "python"  
/// - Node.js: `package.json` → "node"
/// - Go: `go.mod` → "go"
/// - Java: `pom.xml`, `build.gradle`, `build.gradle.kts` → "java"
/// - Lua: `rockspec`, `.luacheckrc`, `init.lua` → "lua"
/// - Default: "default" (when no specific files found)
pub async fn detect_stack(repo_path: &Path) -> Result<String> {
    // Check for language-specific files in priority order
    let stack = if file_exists(repo_path, "Cargo.toml").await {
        "rust"
    } else if file_exists(repo_path, "pyproject.toml").await
        || file_exists(repo_path, "requirements.txt").await
        || file_exists(repo_path, "setup.py").await
    {
        "python"
    } else if file_exists(repo_path, "package.json").await {
        "node"
    } else if file_exists(repo_path, "go.mod").await {
        "go"
    } else if file_exists(repo_path, "pom.xml").await
        || file_exists(repo_path, "build.gradle").await
        || file_exists(repo_path, "build.gradle.kts").await
    {
        "java"
    } else if file_exists(repo_path, "rockspec").await
        || file_exists(repo_path, ".luacheckrc").await
        || file_exists(repo_path, "init.lua").await
    {
        "lua"
    } else {
        "default"
    };

    Ok(stack.to_string())
}

/// Detects the project name from the repository path
///
/// Extracts the directory name from the repository path and cleans it
/// to be suitable for Docker tags by:
/// - Converting to lowercase
/// - Replacing special characters with hyphens
/// - Collapsing consecutive hyphens
/// - Trimming hyphens from both ends
///
/// Falls back to "default" if no name can be extracted
pub async fn detect_project_name(repo_path: &Path) -> Result<String> {
    // Extract the directory name from the repository path
    let project_name = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(clean_project_name)
        .unwrap_or_else(|| "default".to_string());

    // Ensure the name is not empty after cleaning
    let project_name = if project_name.is_empty() {
        "default".to_string()
    } else {
        project_name
    };

    Ok(project_name)
}

/// Checks if a file exists in the repository
async fn file_exists(repo_path: &Path, file_name: &str) -> bool {
    let file_path = repo_path.join(file_name);
    tokio::fs::metadata(&file_path).await.is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_detect_rust_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // Create Cargo.toml file
        tokio::fs::write(temp_dir.path().join("Cargo.toml"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "rust");
    }

    #[tokio::test]
    async fn test_detect_python_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // Create pyproject.toml file
        tokio::fs::write(temp_dir.path().join("pyproject.toml"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "python");
    }

    #[tokio::test]
    async fn test_detect_python_tech_stack_requirements() {
        let temp_dir = TempDir::new().unwrap();

        // Create requirements.txt file
        tokio::fs::write(temp_dir.path().join("requirements.txt"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "python");
    }

    #[tokio::test]
    async fn test_detect_node_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // Create package.json file
        tokio::fs::write(temp_dir.path().join("package.json"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "node");
    }

    #[tokio::test]
    async fn test_detect_go_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // Create go.mod file
        tokio::fs::write(temp_dir.path().join("go.mod"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "go");
    }

    #[tokio::test]
    async fn test_detect_java_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // Create pom.xml file
        tokio::fs::write(temp_dir.path().join("pom.xml"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "java");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_rockspec() {
        let temp_dir = TempDir::new().unwrap();

        // Create rockspec file
        tokio::fs::write(temp_dir.path().join("rockspec"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_luacheckrc() {
        let temp_dir = TempDir::new().unwrap();

        // Create .luacheckrc file
        tokio::fs::write(temp_dir.path().join(".luacheckrc"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_lua_tech_stack_init() {
        let temp_dir = TempDir::new().unwrap();

        // Create init.lua file
        tokio::fs::write(temp_dir.path().join("init.lua"), "")
            .await
            .unwrap();

        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "lua");
    }

    #[tokio::test]
    async fn test_detect_default_tech_stack() {
        let temp_dir = TempDir::new().unwrap();

        // No special files created
        let result = detect_stack(temp_dir.path()).await.unwrap();
        assert_eq!(result, "default");
    }

    #[tokio::test]
    async fn test_detect_project_name() {
        // Create a temp dir with a specific name to test
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("my-awesome-project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "my-awesome-project");
    }

    #[tokio::test]
    async fn test_clean_project_name() {
        // Create a temp dir with special characters in the name
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("My_Awesome Project!");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "my-awesome-project");
    }

    #[tokio::test]
    async fn test_project_name_with_special_chars() {
        // Create a temp dir with special characters
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("test@#$%project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "test-project");
    }

    #[tokio::test]
    async fn test_project_name_fallback() {
        let result = detect_project_name(Path::new("/")).await.unwrap();
        assert_eq!(result, "default");
    }
}
