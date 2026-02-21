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
/// # use std::path::Path;
/// # use std::error::Error;
/// # fn find_repository_root(start_path: &Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
/// #     Ok(start_path.to_path_buf())
/// # }
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
                .into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::test_utils::TestGitRepository;

    #[test]
    fn test_find_repository_root_in_git_repo() {
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init().unwrap();
        let repo_root = test_repo.path();

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
        let test_repo = TestGitRepository::new().unwrap();
        let result = find_repository_root(test_repo.path());

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Not in a git repository")
        );
    }
}
