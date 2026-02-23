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

/// Resolves the actual git directory for a repository path.
///
/// In a normal repository, `.git` is a directory and this returns `repo_path/.git`.
/// In a worktree, `.git` is a file containing `gitdir: <path>` — this parses that
/// and resolves the path (which may be relative) to an absolute path.
pub fn resolve_git_dir(repo_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let git_path = repo_path.join(".git");
    let metadata = std::fs::metadata(&git_path)
        .map_err(|e| format!("Cannot access {}: {}", git_path.display(), e))?;

    if metadata.is_dir() {
        return Ok(git_path);
    }

    // .git is a file — parse gitdir: line
    let content = std::fs::read_to_string(&git_path)
        .map_err(|e| format!("Cannot read {}: {}", git_path.display(), e))?;
    let gitdir_line = content.trim();
    let gitdir_path = gitdir_line.strip_prefix("gitdir: ").ok_or_else(|| {
        format!(
            "Invalid .git file format in {}: {}",
            git_path.display(),
            gitdir_line
        )
    })?;

    let gitdir = Path::new(gitdir_path);
    let resolved = if gitdir.is_absolute() {
        gitdir.to_path_buf()
    } else {
        repo_path.join(gitdir)
    };

    // Canonicalize to resolve any .. components
    resolved
        .canonicalize()
        .map_err(|e| format!("Cannot resolve git dir {}: {}", resolved.display(), e).into())
}

/// Resolves the common git directory (the main repository's `.git`).
///
/// In a normal repository, this is the same as `resolve_git_dir`.
/// In a worktree, the git dir contains a `commondir` file that points to the
/// shared `.git` directory of the main repository.
pub fn resolve_git_common_dir(repo_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let git_dir = resolve_git_dir(repo_path)?;

    let commondir_path = git_dir.join("commondir");
    if commondir_path.is_file() {
        let content = std::fs::read_to_string(&commondir_path)
            .map_err(|e| format!("Cannot read {}: {}", commondir_path.display(), e))?;
        let commondir = content.trim();
        let common = if Path::new(commondir).is_absolute() {
            PathBuf::from(commondir)
        } else {
            git_dir.join(commondir)
        };
        return common
            .canonicalize()
            .map_err(|e| format!("Cannot resolve common dir {}: {}", common.display(), e).into());
    }

    Ok(git_dir)
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

    #[test]
    fn test_resolve_git_dir_normal_repo() {
        let repo = TestGitRepository::new().unwrap();
        repo.init().unwrap();

        let git_dir = resolve_git_dir(repo.path()).unwrap();
        assert!(git_dir.is_dir());
        assert_eq!(git_dir.file_name().unwrap(), ".git");
    }

    #[test]
    fn test_resolve_git_dir_worktree() {
        let repo = TestGitRepository::new().unwrap();
        repo.init_with_commit().unwrap();

        let worktree_tmp = tempfile::TempDir::new().unwrap();
        let worktree_dir = worktree_tmp.path().join("worktree");
        repo.run_git_command(&[
            "worktree",
            "add",
            worktree_dir.to_str().unwrap(),
            "-b",
            "wt-branch",
        ])
        .unwrap();

        let git_dir = resolve_git_dir(&worktree_dir).unwrap();
        // Should resolve to something inside the main repo's .git/worktrees/
        assert!(git_dir.is_dir());
        assert!(git_dir.to_str().unwrap().contains("worktrees"));

        // Clean up worktree before temp dirs are dropped
        std::fs::remove_dir_all(&worktree_dir).ok();
        repo.run_git_command(&["worktree", "prune"]).ok();
    }

    #[test]
    fn test_resolve_git_common_dir_normal_repo() {
        let repo = TestGitRepository::new().unwrap();
        repo.init().unwrap();

        let common_dir = resolve_git_common_dir(repo.path()).unwrap();
        let git_dir = resolve_git_dir(repo.path()).unwrap();
        assert_eq!(common_dir, git_dir);
    }

    #[test]
    fn test_resolve_git_common_dir_worktree() {
        let repo = TestGitRepository::new().unwrap();
        repo.init_with_commit().unwrap();

        let worktree_tmp = tempfile::TempDir::new().unwrap();
        let worktree_dir = worktree_tmp.path().join("worktree");
        repo.run_git_command(&[
            "worktree",
            "add",
            worktree_dir.to_str().unwrap(),
            "-b",
            "wt-common-branch",
        ])
        .unwrap();

        let common_dir = resolve_git_common_dir(&worktree_dir).unwrap();
        let main_git_dir = resolve_git_dir(repo.path()).unwrap();
        // Common dir from worktree should point to main repo's .git
        // Canonicalize both to handle macOS /var -> /private/var symlink
        assert_eq!(
            common_dir.canonicalize().unwrap(),
            main_git_dir.canonicalize().unwrap()
        );

        // Clean up worktree before temp dirs are dropped
        std::fs::remove_dir_all(&worktree_dir).ok();
        repo.run_git_command(&["worktree", "prune"]).ok();
    }

    #[test]
    fn test_resolve_git_dir_not_a_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = resolve_git_dir(temp.path());
        assert!(result.is_err());
    }
}
