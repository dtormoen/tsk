use std::error::Error;
use std::path::{Path, PathBuf};

/// Find the root of a git repository starting from the given path.
///
/// This function walks up the directory tree from the starting path until it finds
/// a directory containing a `.git` subdirectory, which indicates the repository root.
///
/// # Arguments
///
/// * `start_path` - The path to start searching from
///
/// # Returns
///
/// * `Ok(PathBuf)` - The canonical path to the repository root
/// * `Err` - If not in a git repository or if an I/O error occurs
///
/// # Example
///
/// ```no_run
/// use std::path::Path;
/// use tsk::repo_utils::find_repository_root;
///
/// let repo_root = find_repository_root(Path::new(".")).unwrap();
/// println!("Repository root: {}", repo_root.display());
/// ```
pub fn find_repository_root(start_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let start = if start_path.is_relative() {
        std::env::current_dir()?.join(start_path)
    } else {
        start_path.to_path_buf()
    };

    let mut current = start.canonicalize()?;

    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(format!(
                    "Not in a git repository (or any of the parent directories): {}",
                    start_path.display()
                )
                .into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_find_repository_root_in_git_repo() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();

        // Create .git directory
        fs::create_dir(repo_root.join(".git")).unwrap();

        // Create subdirectories
        let sub_dir = repo_root.join("src").join("commands");
        fs::create_dir_all(&sub_dir).unwrap();

        // Test from various locations
        assert_eq!(
            find_repository_root(repo_root).unwrap(),
            repo_root.canonicalize().unwrap()
        );
        assert_eq!(
            find_repository_root(&sub_dir).unwrap(),
            repo_root.canonicalize().unwrap()
        );
        assert_eq!(
            find_repository_root(&repo_root.join("src")).unwrap(),
            repo_root.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_repository_root_not_in_repo() {
        // Create a temporary directory without .git
        let temp_dir = TempDir::new().unwrap();
        let result = find_repository_root(temp_dir.path());

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Not in a git repository"));
    }

    #[test]
    fn test_find_repository_root_with_relative_path() {
        // Create a temporary git repository for testing
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();

        // Create .git directory
        fs::create_dir(repo_root.join(".git")).unwrap();

        // Create a test file
        fs::write(repo_root.join("test.txt"), "test content").unwrap();

        // Change to the temp directory for this test
        let original_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp"));

        if std::env::set_current_dir(repo_root).is_ok() {
            // Test with relative path from inside the repo
            let result = find_repository_root(Path::new("."));
            assert!(result.is_ok());

            let found_root = result.unwrap();
            assert!(found_root.join(".git").exists());

            // Restore original directory
            let _ = std::env::set_current_dir(original_dir);
        } else {
            // If we can't change directory, test with absolute path
            let result = find_repository_root(repo_root);
            assert!(result.is_ok());

            let found_root = result.unwrap();
            assert_eq!(found_root, repo_root.canonicalize().unwrap());
        }
    }
}
