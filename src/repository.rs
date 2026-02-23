use anyhow::Result;
use std::path::Path;

/// Detects the stack based on repository files
///
/// Checks for language-specific marker files in priority order:
/// - Rust: `Cargo.toml` → "rust"
/// - Lua: `rockspec`, `.luacheckrc`, `init.lua` → "lua"
/// - Python: `pyproject.toml`, `requirements.txt`, `setup.py` → "python"
/// - Node.js: `package.json` → "node"
/// - Go: `go.mod` → "go"
/// - Java: `pom.xml`, `build.gradle`, `build.gradle.kts` → "java"
/// - Default: "default" (when no specific files found)
pub async fn detect_stack(repo_path: &Path) -> Result<String> {
    // Check for language-specific files in priority order
    let stack = if file_exists(repo_path, "Cargo.toml").await {
        "rust"
    } else if file_exists(repo_path, "rockspec").await
        || file_exists(repo_path, ".luacheckrc").await
        || file_exists(repo_path, "init.lua").await
    {
        "lua"
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
    // Try to resolve the main repository root via the common git dir.
    // In a worktree, this gives us the main repo's name instead of the worktree dir name.
    let effective_path = match crate::repo_utils::resolve_git_common_dir(repo_path) {
        Ok(common_dir) => {
            // The common dir is the main repo's .git — its parent is the repo root
            common_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| repo_path.to_path_buf())
        }
        Err(_) => repo_path.to_path_buf(),
    };

    let project_name = effective_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(clean_project_name)
        .unwrap_or_else(|| "default".to_string());

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
///
/// Docker image names must be lowercase and can contain:
/// - Lowercase letters (a-z)
/// - Digits (0-9)
/// - Periods (.)
/// - Underscores (_)
/// - Hyphens (-)
///
/// Cannot start or end with a separator (period, underscore, or hyphen)
fn clean_project_name(name: &str) -> String {
    // Convert to lowercase and map characters
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    // Collapse consecutive separators (except for valid patterns like __)
    let mut result = String::new();
    let mut prev_sep = false;
    let mut prev_char = '\0';

    for c in cleaned.chars() {
        let is_separator = c == '-' || c == '_' || c == '.';

        if is_separator {
            // Allow double underscore but not other consecutive separators
            if prev_sep && !(prev_char == '_' && c == '_') {
                continue;
            }
            prev_sep = is_separator;
            prev_char = c;
            result.push(c);
        } else {
            prev_sep = false;
            prev_char = c;
            result.push(c);
        }
    }

    // Trim separators from both ends
    result
        .trim_matches(|c| c == '-' || c == '_' || c == '.')
        .to_string()
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
        assert_eq!(result, "my_awesome-project");
    }

    #[tokio::test]
    async fn test_project_name_with_dots() {
        // Test that dots are preserved (e.g., test.nvim)
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("test.nvim");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "test.nvim");
    }

    #[tokio::test]
    async fn test_project_name_with_underscores() {
        // Test that underscores are preserved
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("my_cool_project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "my_cool_project");
    }

    #[tokio::test]
    async fn test_project_name_with_spaces() {
        // Test that spaces are converted to hyphens
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("my code project");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "my-code-project");
    }

    #[tokio::test]
    async fn test_project_name_mixed_separators() {
        // Test mixed separators
        let temp_base = TempDir::new().unwrap();
        let project_dir = temp_base.path().join("test.nvim-plugin_v2");
        tokio::fs::create_dir(&project_dir).await.unwrap();

        let result = detect_project_name(&project_dir).await.unwrap();
        assert_eq!(result, "test.nvim-plugin_v2");
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

    #[tokio::test]
    async fn test_detect_project_name_from_worktree() {
        let temp_base = TempDir::new().unwrap();
        let main_repo_dir = temp_base.path().join("my-real-project");
        std::fs::create_dir(&main_repo_dir).unwrap();

        let output = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();
        assert!(output.status.success());

        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();

        std::fs::write(main_repo_dir.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();

        // Create worktree with a different name
        let worktree_dir = temp_base.path().join("my-real-project-wt");
        std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_dir.to_str().unwrap(),
                "-b",
                "wt-proj",
            ])
            .current_dir(&main_repo_dir)
            .output()
            .unwrap();

        // Detect project name from worktree — should return main repo's name
        let name = detect_project_name(&worktree_dir).await.unwrap();
        assert_eq!(name, "my-real-project");

        // Clean up
        std::fs::remove_dir_all(&worktree_dir).ok();
    }
}
