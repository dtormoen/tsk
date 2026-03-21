use std::error::Error;
use std::path::{Path, PathBuf};

/// Runs `git -C <path> rev-parse <args>` and returns the trimmed stdout.
fn run_git_rev_parse(path: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run git rev-parse: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git rev-parse failed in {}: {}",
            path.display(),
            stderr.trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parsed output from `git rev-parse --show-toplevel --git-dir --git-common-dir`.
struct GitRepoInfo {
    toplevel: PathBuf,
    is_worktree: bool,
    /// The canonicalized `--git-common-dir` path, resolved relative to the query path.
    common_dir: PathBuf,
}

/// Detects git repository info for a given start path.
///
/// Runs a single `git rev-parse` invocation with `--show-toplevel --git-dir --git-common-dir`
/// and returns parsed results. A worktree is detected when `--git-dir` differs from
/// `--git-common-dir`.
fn detect_git_repo(start_path: &Path) -> Result<GitRepoInfo, Box<dyn Error>> {
    let start = if start_path.is_relative() {
        std::env::current_dir()?.join(start_path)
    } else {
        start_path.to_path_buf()
    };

    let canonical = start.canonicalize()?;

    let output = run_git_rev_parse(
        &canonical,
        &["--show-toplevel", "--git-dir", "--git-common-dir"],
    )
    .map_err(|e| {
        format!(
            "Not in a git repository (or any of the parent directories): {} ({e})",
            start_path.display()
        )
    })?;

    let lines: Vec<&str> = output.lines().collect();
    if lines.len() < 3 {
        return Err(format!(
            "Unexpected git rev-parse output in {}: {output}",
            canonical.display()
        )
        .into());
    }

    let toplevel = lines[0];
    let git_dir = lines[1];
    let git_common_dir = lines[2];
    let is_worktree = git_dir != git_common_dir;

    let common_path = if Path::new(git_common_dir).is_absolute() {
        PathBuf::from(git_common_dir)
    } else {
        canonical.join(git_common_dir)
    };
    let common_dir = common_path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve common dir {}: {e}", common_path.display()))?;

    Ok(GitRepoInfo {
        toplevel: PathBuf::from(toplevel),
        is_worktree,
        common_dir,
    })
}

/// Find the root of a git repository starting from the given path.
///
/// Uses `git rev-parse` to locate the repository root. For git worktrees,
/// the main repository root is resolved by taking the parent of the common
/// git directory.
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
    let info = detect_git_repo(start_path)?;

    if info.is_worktree
        && let Some(main_root) = info.common_dir.parent()
    {
        return Ok(main_root.to_path_buf());
    }

    Ok(info.toplevel)
}

/// Resolves the common git directory (the main repository's `.git`).
///
/// Uses `git rev-parse --git-common-dir` to find the shared `.git` directory.
/// In a normal repository, this is the repo's `.git` directory.
/// In a worktree, this points to the main repository's `.git` directory.
pub fn resolve_git_common_dir(repo_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let result = run_git_rev_parse(repo_path, &["--git-common-dir"])?;
    let common_dir = Path::new(&result);
    let resolved = if common_dir.is_absolute() {
        common_dir.to_path_buf()
    } else {
        repo_path.join(common_dir)
    };
    resolved
        .canonicalize()
        .map_err(|e| format!("Cannot resolve common dir {}: {e}", resolved.display()).into())
}

/// Find the worktree root directory from a starting path.
///
/// Uses `git rev-parse` to determine if the path is inside a git worktree.
/// Returns the worktree directory if so, or `None` if it's a normal repository.
pub fn find_worktree_root(start_path: &Path) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let info = detect_git_repo(start_path)?;

    if info.is_worktree {
        Ok(Some(info.toplevel))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::TestGitRepository;

    #[test]
    fn test_find_repository_root_in_git_repo() {
        let test_repo = TestGitRepository::new().unwrap();
        test_repo.init().unwrap();
        let repo_root = test_repo.path();

        // Create subdirectories
        let sub_dir = repo_root.join("src").join("commands");
        std::fs::create_dir_all(&sub_dir).unwrap();

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
    fn test_resolve_git_common_dir_normal_repo() {
        let repo = TestGitRepository::new().unwrap();
        repo.init().unwrap();

        let common_dir = resolve_git_common_dir(repo.path()).unwrap();
        assert!(common_dir.is_dir());
        assert_eq!(common_dir.file_name().unwrap(), ".git");
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
        let main_common_dir = resolve_git_common_dir(repo.path()).unwrap();
        // Common dir from worktree should point to main repo's .git
        // Canonicalize both to handle macOS /var -> /private/var symlink
        assert_eq!(
            common_dir.canonicalize().unwrap(),
            main_common_dir.canonicalize().unwrap()
        );

        // Clean up worktree before temp dirs are dropped
        std::fs::remove_dir_all(&worktree_dir).ok();
        repo.run_git_command(&["worktree", "prune"]).ok();
    }

    #[test]
    fn test_find_repository_root_from_worktree() {
        let repo = TestGitRepository::new().unwrap();
        repo.init_with_commit().unwrap();

        let worktree_tmp = tempfile::TempDir::new().unwrap();
        let worktree_dir = worktree_tmp.path().join("worktree");
        repo.run_git_command(&[
            "worktree",
            "add",
            worktree_dir.to_str().unwrap(),
            "-b",
            "wt-find-root-branch",
        ])
        .unwrap();

        // From the worktree root, should resolve to main repo root
        let found_root = find_repository_root(&worktree_dir).unwrap();
        assert_eq!(
            found_root.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );

        // From a subdirectory within the worktree, should also resolve to main repo root
        let sub_dir = worktree_dir.join("subdir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let found_root_from_sub = find_repository_root(&sub_dir).unwrap();
        assert_eq!(
            found_root_from_sub.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );

        // Clean up worktree before temp dirs are dropped
        std::fs::remove_dir_all(&worktree_dir).ok();
        repo.run_git_command(&["worktree", "prune"]).ok();
    }

    #[test]
    fn test_find_worktree_root_normal_repo() {
        let repo = TestGitRepository::new().unwrap();
        repo.init_with_commit().unwrap();

        // Normal repo should return None
        let result = find_worktree_root(repo.path()).unwrap();
        assert!(result.is_none(), "Normal repo should not be a worktree");
    }

    #[test]
    fn test_find_worktree_root_from_worktree() {
        let repo = TestGitRepository::new().unwrap();
        repo.init_with_commit().unwrap();

        let worktree_tmp = tempfile::TempDir::new().unwrap();
        let worktree_dir = worktree_tmp.path().join("worktree");
        repo.run_git_command(&[
            "worktree",
            "add",
            worktree_dir.to_str().unwrap(),
            "-b",
            "wt-detect-branch",
        ])
        .unwrap();

        // From worktree root, should return the worktree path
        let result = find_worktree_root(&worktree_dir).unwrap();
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            worktree_dir.canonicalize().unwrap()
        );

        // From a subdirectory within the worktree, should still return the worktree root
        let sub_dir = worktree_dir.join("subdir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let result = find_worktree_root(&sub_dir).unwrap();
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            worktree_dir.canonicalize().unwrap()
        );

        // Clean up worktree before temp dirs are dropped
        std::fs::remove_dir_all(&worktree_dir).ok();
        repo.run_git_command(&["worktree", "prune"]).ok();
    }

    #[test]
    fn test_resolve_git_common_dir_not_a_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = resolve_git_common_dir(temp.path());
        assert!(result.is_err());
    }
}
