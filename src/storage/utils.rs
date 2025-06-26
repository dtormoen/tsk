use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Generate a stable hash for a repository path
pub fn get_repo_hash(repo_path: &Path) -> String {
    let canonical = repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let path_str = canonical.to_string_lossy();

    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();

    // Convert to hex string, take first 8 characters for brevity
    format!("{hash:x}").chars().take(8).collect()
}
