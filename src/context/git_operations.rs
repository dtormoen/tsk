use async_trait::async_trait;
use git2::{Repository, RepositoryOpenFlags};
use std::path::{Path, PathBuf};

#[async_trait]
pub trait GitOperations: Send + Sync {
    /// Check if the given path is within a git repository
    async fn is_git_repository(&self, repo_path: &Path) -> Result<bool, String>;

    async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String>;

    async fn get_status(&self, repo_path: &Path) -> Result<String, String>;

    async fn add_all(&self, repo_path: &Path) -> Result<(), String>;

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String>;

    async fn add_remote(
        &self,
        repo_path: &Path,
        remote_name: &str,
        url: &str,
    ) -> Result<(), String>;

    async fn fetch_branch(
        &self,
        repo_path: &Path,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), String>;

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String>;

    /// Check if a branch has commits that are not in the base branch
    async fn has_commits_not_in_base(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<bool, String>;

    /// Delete a branch
    async fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String>;

    /// Get the current commit SHA
    async fn get_current_commit(&self, repo_path: &Path) -> Result<String, String>;

    /// Create a branch from a specific commit
    async fn create_branch_from_commit(
        &self,
        repo_path: &Path,
        branch_name: &str,
        commit_sha: &str,
    ) -> Result<(), String>;

    /// Get list of tracked files in the repository
    async fn get_tracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String>;

    /// Get list of untracked files that are not ignored
    async fn get_untracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String>;
}

pub struct DefaultGitOperations;

impl DefaultGitOperations {}

#[async_trait]
impl GitOperations for DefaultGitOperations {
    async fn is_git_repository(&self, repo_path: &Path) -> Result<bool, String> {
        match Repository::open_ext(repo_path, RepositoryOpenFlags::empty(), &[] as &[&Path]) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {e}"))?;

                let commit = head
                    .peel_to_commit()
                    .map_err(|e| format!("Failed to get commit from HEAD: {e}"))?;

                repo.branch(&branch_name, &commit, false)
                    .map_err(|e| format!("Failed to create branch: {e}"))?;

                repo.set_head(&format!("refs/heads/{branch_name}"))
                    .map_err(|e| format!("Failed to checkout branch: {e}"))?;

                repo.checkout_head(None)
                    .map_err(|e| format!("Failed to update working directory: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_status(&self, repo_path: &Path) -> Result<String, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<String, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let statuses = repo
                    .statuses(None)
                    .map_err(|e| format!("Failed to get repository status: {e}"))?;

                let mut result = String::new();

                for entry in statuses.iter() {
                    let status = entry.status();
                    if let Some(path) = entry.path() {
                        let status_char = if status.is_wt_new() {
                            "??"
                        } else if status.contains(git2::Status::INDEX_NEW) {
                            "A"
                        } else if status.contains(git2::Status::INDEX_MODIFIED)
                            || status.contains(git2::Status::WT_MODIFIED)
                        {
                            "M"
                        } else if status.contains(git2::Status::INDEX_DELETED)
                            || status.contains(git2::Status::WT_DELETED)
                        {
                            "D"
                        } else if status.contains(git2::Status::INDEX_RENAMED)
                            || status.contains(git2::Status::WT_RENAMED)
                        {
                            "R"
                        } else if status.contains(git2::Status::CONFLICTED) {
                            "C"
                        } else {
                            continue;
                        };

                        result.push_str(&format!("{status_char} {path}\n"));
                    }
                }

                Ok(result)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn add_all(&self, repo_path: &Path) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {e}"))?;

                index
                    .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                    .map_err(|e| format!("Failed to add files to index: {e}"))?;

                index
                    .write()
                    .map_err(|e| format!("Failed to write index: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let message = message.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {e}"))?;

                let tree_id = index
                    .write_tree()
                    .map_err(|e| format!("Failed to write tree: {e}"))?;

                let tree = repo
                    .find_tree(tree_id)
                    .map_err(|e| format!("Failed to find tree: {e}"))?;

                let signature = repo
                    .signature()
                    .map_err(|e| format!("Failed to get signature: {e}"))?;

                let parent_commit = match repo.head() {
                    Ok(head) => Some(
                        head.peel_to_commit()
                            .map_err(|e| format!("Failed to get parent commit: {e}"))?,
                    ),
                    Err(_) => None,
                };

                let parents = if let Some(ref parent) = parent_commit {
                    vec![parent]
                } else {
                    vec![]
                };

                repo.commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    &parents,
                )
                .map_err(|e| format!("Failed to create commit: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn add_remote(
        &self,
        repo_path: &Path,
        remote_name: &str,
        url: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            let url = url.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let result = repo.remote(&remote_name, &url);
                match result {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        if e.code() == git2::ErrorCode::Exists {
                            Ok(())
                        } else {
                            Err(format!("Failed to add remote: {e}"))
                        }
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn fetch_branch(
        &self,
        repo_path: &Path,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut remote = repo
                    .find_remote(&remote_name)
                    .map_err(|e| format!("Failed to find remote: {e}"))?;

                let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");

                remote
                    .fetch(&[&refspec], None, None)
                    .map_err(|e| format!("Failed to fetch changes: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                repo.remote_delete(&remote_name)
                    .map_err(|e| format!("Failed to remove temporary remote: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn has_commits_not_in_base(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<bool, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            let base_branch = base_branch.to_owned();
            move || -> Result<bool, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                // Get the branch reference
                let branch_ref = format!("refs/heads/{branch_name}");
                let branch = repo
                    .find_reference(&branch_ref)
                    .map_err(|e| format!("Failed to find branch {branch_name}: {e}"))?;

                let branch_oid = branch
                    .target()
                    .ok_or_else(|| format!("Branch {branch_name} has no target"))?;

                // Get the base branch reference
                let base_ref = format!("refs/heads/{base_branch}");
                let base = repo
                    .find_reference(&base_ref)
                    .map_err(|e| format!("Failed to find base branch {base_branch}: {e}"))?;

                let base_oid = base
                    .target()
                    .ok_or_else(|| format!("Base branch {base_branch} has no target"))?;

                // If they point to the same commit, there are no unique commits
                if branch_oid == base_oid {
                    return Ok(false);
                }

                // Check if the branch commit is reachable from the base branch
                // If it is, then there are no unique commits in the branch
                match repo.graph_descendant_of(base_oid, branch_oid) {
                    Ok(true) => Ok(false), // branch is behind base, no unique commits
                    Ok(false) => Ok(true), // branch has commits not in base
                    Err(_) => {
                        // If we can't determine the relationship, assume there are commits
                        // This is safer than assuming there aren't
                        Ok(true)
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut branch = repo
                    .find_branch(&branch_name, git2::BranchType::Local)
                    .map_err(|e| format!("Failed to find branch {branch_name}: {e}"))?;

                branch
                    .delete()
                    .map_err(|e| format!("Failed to delete branch {branch_name}: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_current_commit(&self, repo_path: &Path) -> Result<String, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<String, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {e}"))?;

                let commit = head
                    .peel_to_commit()
                    .map_err(|e| format!("Failed to get commit from HEAD: {e}"))?;

                Ok(commit.id().to_string())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn create_branch_from_commit(
        &self,
        repo_path: &Path,
        branch_name: &str,
        commit_sha: &str,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let branch_name = branch_name.to_owned();
            let commit_sha = commit_sha.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let oid = git2::Oid::from_str(&commit_sha)
                    .map_err(|e| format!("Invalid commit SHA: {e}"))?;

                let commit = repo
                    .find_commit(oid)
                    .map_err(|e| format!("Failed to find commit {commit_sha}: {e}"))?;

                repo.branch(&branch_name, &commit, false)
                    .map_err(|e| format!("Failed to create branch: {e}"))?;

                repo.set_head(&format!("refs/heads/{branch_name}"))
                    .map_err(|e| format!("Failed to checkout branch: {e}"))?;

                // Force update the working directory to match the commit
                let mut checkout_opts = git2::build::CheckoutBuilder::new();
                checkout_opts.force();
                repo.checkout_head(Some(&mut checkout_opts))
                    .map_err(|e| format!("Failed to update working directory: {e}"))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_tracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<Vec<PathBuf>, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {e}"))?;

                let tree = head
                    .peel_to_tree()
                    .map_err(|e| format!("Failed to get tree from HEAD: {e}"))?;

                let mut tracked_files = Vec::new();

                tree.walk(git2::TreeWalkMode::PreOrder, |path, entry| {
                    if entry.kind() == Some(git2::ObjectType::Blob) {
                        let file_path = if path.is_empty() {
                            PathBuf::from(entry.name().unwrap_or(""))
                        } else {
                            PathBuf::from(path).join(entry.name().unwrap_or(""))
                        };
                        tracked_files.push(file_path);
                    }
                    git2::TreeWalkResult::Ok
                })
                .map_err(|e| format!("Failed to walk tree: {e}"))?;

                Ok(tracked_files)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }

    async fn get_untracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<Vec<PathBuf>, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {e}"))?;

                let mut opts = git2::StatusOptions::new();
                opts.include_untracked(true).include_ignored(false);

                let statuses = repo
                    .statuses(Some(&mut opts))
                    .map_err(|e| format!("Failed to get repository status: {e}"))?;

                let mut untracked_files = Vec::new();

                for entry in statuses.iter() {
                    let status = entry.status();
                    // Check if file is untracked (not in index)
                    if status.is_wt_new() {
                        if let Some(path) = entry.path() {
                            untracked_files.push(PathBuf::from(path));
                        }
                    }
                }

                Ok(untracked_files)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub struct MockGitOperations {
        is_repo_result: Arc<Mutex<Result<bool, String>>>,
        create_branch_calls: Arc<Mutex<Vec<(String, String)>>>,
        create_branch_result: Arc<Mutex<Result<(), String>>>,
        get_status_calls: Arc<Mutex<Vec<String>>>,
        get_status_result: Arc<Mutex<Result<String, String>>>,
        add_all_calls: Arc<Mutex<Vec<String>>>,
        add_all_result: Arc<Mutex<Result<(), String>>>,
        commit_calls: Arc<Mutex<Vec<(String, String)>>>,
        commit_result: Arc<Mutex<Result<(), String>>>,
        add_remote_calls: Arc<Mutex<Vec<(String, String, String)>>>,
        add_remote_result: Arc<Mutex<Result<(), String>>>,
        fetch_branch_calls: Arc<Mutex<Vec<(String, String, String)>>>,
        fetch_branch_result: Arc<Mutex<Result<(), String>>>,
        remove_remote_calls: Arc<Mutex<Vec<(String, String)>>>,
        remove_remote_result: Arc<Mutex<Result<(), String>>>,
        has_commits_not_in_base_calls: Arc<Mutex<Vec<(String, String, String)>>>,
        has_commits_not_in_base_result: Arc<Mutex<Result<bool, String>>>,
        delete_branch_calls: Arc<Mutex<Vec<(String, String)>>>,
        delete_branch_result: Arc<Mutex<Result<(), String>>>,
        get_current_commit_calls: Arc<Mutex<Vec<String>>>,
        get_current_commit_result: Arc<Mutex<Result<String, String>>>,
        create_branch_from_commit_calls: Arc<Mutex<Vec<(String, String, String)>>>,
        create_branch_from_commit_result: Arc<Mutex<Result<(), String>>>,
        get_tracked_files_calls: Arc<Mutex<Vec<String>>>,
        get_tracked_files_result: Arc<Mutex<Result<Vec<PathBuf>, String>>>,
        get_untracked_files_calls: Arc<Mutex<Vec<String>>>,
        get_untracked_files_result: Arc<Mutex<Result<Vec<PathBuf>, String>>>,
    }

    impl MockGitOperations {
        pub fn new() -> Self {
            Self {
                is_repo_result: Arc::new(Mutex::new(Ok(true))),
                create_branch_calls: Arc::new(Mutex::new(Vec::new())),
                create_branch_result: Arc::new(Mutex::new(Ok(()))),
                get_status_calls: Arc::new(Mutex::new(Vec::new())),
                get_status_result: Arc::new(Mutex::new(Ok("".to_string()))),
                add_all_calls: Arc::new(Mutex::new(Vec::new())),
                add_all_result: Arc::new(Mutex::new(Ok(()))),
                commit_calls: Arc::new(Mutex::new(Vec::new())),
                commit_result: Arc::new(Mutex::new(Ok(()))),
                add_remote_calls: Arc::new(Mutex::new(Vec::new())),
                add_remote_result: Arc::new(Mutex::new(Ok(()))),
                fetch_branch_calls: Arc::new(Mutex::new(Vec::new())),
                fetch_branch_result: Arc::new(Mutex::new(Ok(()))),
                remove_remote_calls: Arc::new(Mutex::new(Vec::new())),
                remove_remote_result: Arc::new(Mutex::new(Ok(()))),
                has_commits_not_in_base_calls: Arc::new(Mutex::new(Vec::new())),
                has_commits_not_in_base_result: Arc::new(Mutex::new(Ok(true))),
                delete_branch_calls: Arc::new(Mutex::new(Vec::new())),
                delete_branch_result: Arc::new(Mutex::new(Ok(()))),
                get_current_commit_calls: Arc::new(Mutex::new(Vec::new())),
                get_current_commit_result: Arc::new(Mutex::new(Ok(
                    "abc123def456789012345678901234567890abcd".to_string(),
                ))),
                create_branch_from_commit_calls: Arc::new(Mutex::new(Vec::new())),
                create_branch_from_commit_result: Arc::new(Mutex::new(Ok(()))),
                get_tracked_files_calls: Arc::new(Mutex::new(Vec::new())),
                get_tracked_files_result: Arc::new(Mutex::new(Ok(vec![]))),
                get_untracked_files_calls: Arc::new(Mutex::new(Vec::new())),
                get_untracked_files_result: Arc::new(Mutex::new(Ok(vec![]))),
            }
        }

        pub fn set_is_repo_result(&self, result: Result<bool, String>) {
            *self.is_repo_result.lock().unwrap() = result;
        }

        pub fn set_get_status_result(&self, result: Result<String, String>) {
            *self.get_status_result.lock().unwrap() = result;
        }

        pub fn get_create_branch_calls(&self) -> Vec<(String, String)> {
            self.create_branch_calls.lock().unwrap().clone()
        }

        pub fn get_get_status_calls(&self) -> Vec<String> {
            self.get_status_calls.lock().unwrap().clone()
        }

        pub fn get_add_all_calls(&self) -> Vec<String> {
            self.add_all_calls.lock().unwrap().clone()
        }

        pub fn get_commit_calls(&self) -> Vec<(String, String)> {
            self.commit_calls.lock().unwrap().clone()
        }

        pub fn get_add_remote_calls(&self) -> Vec<(String, String, String)> {
            self.add_remote_calls.lock().unwrap().clone()
        }

        pub fn get_fetch_branch_calls(&self) -> Vec<(String, String, String)> {
            self.fetch_branch_calls.lock().unwrap().clone()
        }

        pub fn get_remove_remote_calls(&self) -> Vec<(String, String)> {
            self.remove_remote_calls.lock().unwrap().clone()
        }

        pub fn set_has_commits_not_in_base_result(&self, result: Result<bool, String>) {
            *self.has_commits_not_in_base_result.lock().unwrap() = result;
        }

        pub fn get_delete_branch_calls(&self) -> Vec<(String, String)> {
            self.delete_branch_calls.lock().unwrap().clone()
        }

        pub fn set_get_current_commit_result(&self, result: Result<String, String>) {
            *self.get_current_commit_result.lock().unwrap() = result;
        }

        pub fn get_get_current_commit_calls(&self) -> Vec<String> {
            self.get_current_commit_calls.lock().unwrap().clone()
        }

        pub fn get_create_branch_from_commit_calls(&self) -> Vec<(String, String, String)> {
            self.create_branch_from_commit_calls.lock().unwrap().clone()
        }

        pub fn set_get_tracked_files_result(&self, result: Result<Vec<PathBuf>, String>) {
            *self.get_tracked_files_result.lock().unwrap() = result;
        }

        pub fn set_get_untracked_files_result(&self, result: Result<Vec<PathBuf>, String>) {
            *self.get_untracked_files_result.lock().unwrap() = result;
        }
    }

    #[async_trait]
    impl GitOperations for MockGitOperations {
        async fn is_git_repository(&self, _repo_path: &Path) -> Result<bool, String> {
            self.is_repo_result.lock().unwrap().clone()
        }

        async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
            self.create_branch_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                branch_name.to_string(),
            ));
            self.create_branch_result.lock().unwrap().clone()
        }

        async fn get_status(&self, repo_path: &Path) -> Result<String, String> {
            self.get_status_calls
                .lock()
                .unwrap()
                .push(repo_path.to_string_lossy().to_string());
            self.get_status_result.lock().unwrap().clone()
        }

        async fn add_all(&self, repo_path: &Path) -> Result<(), String> {
            self.add_all_calls
                .lock()
                .unwrap()
                .push(repo_path.to_string_lossy().to_string());
            self.add_all_result.lock().unwrap().clone()
        }

        async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String> {
            self.commit_calls
                .lock()
                .unwrap()
                .push((repo_path.to_string_lossy().to_string(), message.to_string()));
            self.commit_result.lock().unwrap().clone()
        }

        async fn add_remote(
            &self,
            repo_path: &Path,
            remote_name: &str,
            url: &str,
        ) -> Result<(), String> {
            self.add_remote_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                remote_name.to_string(),
                url.to_string(),
            ));
            self.add_remote_result.lock().unwrap().clone()
        }

        async fn fetch_branch(
            &self,
            repo_path: &Path,
            remote_name: &str,
            branch_name: &str,
        ) -> Result<(), String> {
            self.fetch_branch_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                remote_name.to_string(),
                branch_name.to_string(),
            ));
            self.fetch_branch_result.lock().unwrap().clone()
        }

        async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String> {
            self.remove_remote_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                remote_name.to_string(),
            ));
            self.remove_remote_result.lock().unwrap().clone()
        }

        async fn has_commits_not_in_base(
            &self,
            repo_path: &Path,
            branch_name: &str,
            base_branch: &str,
        ) -> Result<bool, String> {
            self.has_commits_not_in_base_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                branch_name.to_string(),
                base_branch.to_string(),
            ));
            self.has_commits_not_in_base_result.lock().unwrap().clone()
        }

        async fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
            self.delete_branch_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                branch_name.to_string(),
            ));
            self.delete_branch_result.lock().unwrap().clone()
        }

        async fn get_current_commit(&self, repo_path: &Path) -> Result<String, String> {
            self.get_current_commit_calls
                .lock()
                .unwrap()
                .push(repo_path.to_string_lossy().to_string());
            self.get_current_commit_result.lock().unwrap().clone()
        }

        async fn create_branch_from_commit(
            &self,
            repo_path: &Path,
            branch_name: &str,
            commit_sha: &str,
        ) -> Result<(), String> {
            self.create_branch_from_commit_calls.lock().unwrap().push((
                repo_path.to_string_lossy().to_string(),
                branch_name.to_string(),
                commit_sha.to_string(),
            ));
            self.create_branch_from_commit_result
                .lock()
                .unwrap()
                .clone()
        }

        async fn get_tracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String> {
            self.get_tracked_files_calls
                .lock()
                .unwrap()
                .push(repo_path.to_string_lossy().to_string());
            self.get_tracked_files_result.lock().unwrap().clone()
        }

        async fn get_untracked_files(&self, repo_path: &Path) -> Result<Vec<PathBuf>, String> {
            self.get_untracked_files_calls
                .lock()
                .unwrap()
                .push(repo_path.to_string_lossy().to_string());
            self.get_untracked_files_result.lock().unwrap().clone()
        }
    }

    #[cfg(test)]
    mod integration_tests {
        use super::*;
        use std::path::Path;
        use tempfile::TempDir;

        #[tokio::test]
        async fn test_is_git_repository() {
            let git_ops = DefaultGitOperations;

            // Test with a directory that is not a git repository
            let non_git_dir = TempDir::new().unwrap();
            let is_repo = git_ops.is_git_repository(non_git_dir.path()).await.unwrap();
            assert!(!is_repo, "Non-git directory should return false");

            // Test with a valid git repository
            let git_dir = TempDir::new().unwrap();
            git2::Repository::init(git_dir.path()).unwrap();
            let is_repo = git_ops.is_git_repository(git_dir.path()).await.unwrap();
            assert!(is_repo, "Git repository should return true");

            // Test with a subdirectory inside a git repository
            let subdir = git_dir.path().join("subdir");
            std::fs::create_dir(&subdir).unwrap();
            let is_repo = git_ops.is_git_repository(&subdir).await.unwrap();
            assert!(
                is_repo,
                "Subdirectory inside git repository should return true"
            );
        }

        #[tokio::test]
        async fn test_default_git_operations_with_real_repo() {
            let git_ops = DefaultGitOperations;
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path();

            // Initialize a real git repository
            let repo = git2::Repository::init(repo_path).unwrap();

            // Test get_status on empty repo
            let status = git_ops.get_status(repo_path).await.unwrap();
            assert_eq!(status, "");

            // Create a test file
            std::fs::write(repo_path.join("test.txt"), "Hello, world!").unwrap();

            // Test get_status with untracked file
            let status = git_ops.get_status(repo_path).await.unwrap();
            assert!(status.contains("?? test.txt"));

            // Test add_all
            git_ops.add_all(repo_path).await.unwrap();

            // Test get_status after add
            let status = git_ops.get_status(repo_path).await.unwrap();
            assert!(status.contains("A test.txt"));

            // Configure git user for commit
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();

            // Test commit
            git_ops.commit(repo_path, "Initial commit").await.unwrap();

            // Test get_status after commit
            let status = git_ops.get_status(repo_path).await.unwrap();
            assert_eq!(status, "");

            // Test create_branch
            git_ops
                .create_branch(repo_path, "test-branch")
                .await
                .unwrap();

            // Verify we're on the new branch
            let head = repo.head().unwrap();
            let branch_name = head.shorthand().unwrap();
            assert_eq!(branch_name, "test-branch");
        }

        #[tokio::test]
        async fn test_default_git_operations_remotes() {
            let git_ops = DefaultGitOperations;
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path();

            // Initialize a git repository
            git2::Repository::init(repo_path).unwrap();

            // Test add_remote
            git_ops
                .add_remote(repo_path, "origin", "https://github.com/test/repo.git")
                .await
                .unwrap();

            // Test adding the same remote again (should not error)
            git_ops
                .add_remote(repo_path, "origin", "https://github.com/test/repo.git")
                .await
                .unwrap();

            // Test remove_remote
            git_ops.remove_remote(repo_path, "origin").await.unwrap();
        }

        #[tokio::test]
        async fn test_mock_git_operations() {
            use super::tests::MockGitOperations;
            let mock = MockGitOperations::new();

            // Test is_git_repository
            mock.set_is_repo_result(Ok(true));
            assert_eq!(
                mock.is_git_repository(Path::new("/test")).await.unwrap(),
                true
            );

            mock.set_is_repo_result(Ok(false));
            assert_eq!(
                mock.is_git_repository(Path::new("/test")).await.unwrap(),
                false
            );

            // Test get_status
            mock.set_get_status_result(Ok("M file.txt\n".to_string()));
            let status = mock.get_status(Path::new("/test")).await.unwrap();
            assert_eq!(status, "M file.txt\n");

            // Test create_branch
            let result = mock
                .create_branch(Path::new("/test"), "feature-branch")
                .await;
            assert!(result.is_ok());

            // Verify the call was recorded
            let calls = mock.get_create_branch_calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0],
                ("/test".to_string(), "feature-branch".to_string())
            );
        }

        #[tokio::test]
        async fn test_mock_git_operations_commit_flow() {
            use super::tests::MockGitOperations;
            let mock = MockGitOperations::new();

            let repo_path = Path::new("/test/repo");

            // Simulate a commit flow
            mock.set_get_status_result(Ok("M file.txt\n".to_string()));
            let status = mock.get_status(repo_path).await.unwrap();
            assert!(!status.is_empty());

            // Add all changes
            let result = mock.add_all(repo_path).await;
            assert!(result.is_ok());

            // Commit
            let result = mock.commit(repo_path, "Test commit message").await;
            assert!(result.is_ok());

            // Verify calls were recorded
            assert_eq!(mock.get_get_status_calls().len(), 1);
            assert_eq!(mock.get_add_all_calls().len(), 1);
            assert_eq!(mock.get_commit_calls().len(), 1);

            let commit_calls = mock.get_commit_calls();
            assert_eq!(commit_calls[0].1, "Test commit message");
        }

        #[tokio::test]
        async fn test_mock_git_operations_remote_operations() {
            use super::tests::MockGitOperations;
            let mock = MockGitOperations::new();

            let repo_path = Path::new("/test/repo");
            let remote_name = "origin";
            let remote_url = "https://github.com/test/repo.git";
            let branch_name = "main";

            // Add remote
            let result = mock.add_remote(repo_path, remote_name, remote_url).await;
            assert!(result.is_ok());

            // Fetch branch
            let result = mock.fetch_branch(repo_path, remote_name, branch_name).await;
            assert!(result.is_ok());

            // Remove remote
            let result = mock.remove_remote(repo_path, remote_name).await;
            assert!(result.is_ok());

            // Verify calls
            let add_remote_calls = mock.get_add_remote_calls();
            assert_eq!(add_remote_calls.len(), 1);
            assert_eq!(add_remote_calls[0].1, remote_name);
            assert_eq!(add_remote_calls[0].2, remote_url);

            let fetch_calls = mock.get_fetch_branch_calls();
            assert_eq!(fetch_calls.len(), 1);
            assert_eq!(fetch_calls[0].1, remote_name);
            assert_eq!(fetch_calls[0].2, branch_name);

            let remove_calls = mock.get_remove_remote_calls();
            assert_eq!(remove_calls.len(), 1);
            assert_eq!(remove_calls[0].1, remote_name);
        }

        #[tokio::test]
        async fn test_get_current_commit() {
            let git_ops = DefaultGitOperations;
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path();

            // Initialize a real git repository
            let repo = git2::Repository::init(repo_path).unwrap();

            // Configure git user for commit
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();

            // Create and commit a file
            std::fs::write(repo_path.join("test.txt"), "Hello, world!").unwrap();
            git_ops.add_all(repo_path).await.unwrap();
            git_ops.commit(repo_path, "Initial commit").await.unwrap();

            // Get the current commit
            let commit_sha = git_ops.get_current_commit(repo_path).await.unwrap();
            assert!(!commit_sha.is_empty());
            assert_eq!(commit_sha.len(), 40); // SHA should be 40 characters

            // Verify it's the same as what git2 reports
            let head = repo.head().unwrap();
            let head_commit = head.peel_to_commit().unwrap();
            assert_eq!(commit_sha, head_commit.id().to_string());
        }

        #[tokio::test]
        async fn test_create_branch_from_commit() {
            let git_ops = DefaultGitOperations;
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path();

            // Initialize a real git repository
            let repo = git2::Repository::init(repo_path).unwrap();

            // Configure git user for commit
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();

            // Create first commit
            std::fs::write(repo_path.join("file1.txt"), "First file").unwrap();
            git_ops.add_all(repo_path).await.unwrap();
            git_ops.commit(repo_path, "First commit").await.unwrap();

            // Get the first commit SHA
            let first_commit_sha = git_ops.get_current_commit(repo_path).await.unwrap();

            // Create second commit
            std::fs::write(repo_path.join("file2.txt"), "Second file").unwrap();
            git_ops.add_all(repo_path).await.unwrap();
            git_ops.commit(repo_path, "Second commit").await.unwrap();

            // Create a branch from the first commit
            git_ops
                .create_branch_from_commit(repo_path, "feature-from-first", &first_commit_sha)
                .await
                .unwrap();

            // Verify we're on the new branch
            let head = repo.head().unwrap();
            let branch_name = head.shorthand().unwrap();
            assert_eq!(branch_name, "feature-from-first");

            // Verify the branch is at the first commit
            let current_commit = head.peel_to_commit().unwrap();
            assert_eq!(current_commit.id().to_string(), first_commit_sha);

            // Verify the second file doesn't exist in the working directory
            assert!(!repo_path.join("file2.txt").exists());
            assert!(repo_path.join("file1.txt").exists());
        }

        #[tokio::test]
        async fn test_mock_get_current_commit() {
            use super::tests::MockGitOperations;
            let mock = MockGitOperations::new();

            // Test get_current_commit
            let commit_sha = mock.get_current_commit(Path::new("/test")).await.unwrap();
            assert_eq!(commit_sha, "abc123def456789012345678901234567890abcd");

            // Verify the call was recorded
            let calls = mock.get_get_current_commit_calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0], "/test");

            // Test with error
            mock.set_get_current_commit_result(Err("Failed to get commit".to_string()));
            let result = mock.get_current_commit(Path::new("/test2")).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_get_untracked_files() {
            let git_ops = DefaultGitOperations;
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path();

            // Initialize a real git repository
            let repo = git2::Repository::init(repo_path).unwrap();

            // Configure git user for commit
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();

            // Create and add some tracked files
            std::fs::write(repo_path.join("tracked.txt"), "tracked content").unwrap();
            git_ops.add_all(repo_path).await.unwrap();
            git_ops.commit(repo_path, "Initial commit").await.unwrap();

            // Create untracked files
            std::fs::write(repo_path.join("untracked1.txt"), "untracked content 1").unwrap();
            std::fs::write(repo_path.join("untracked2.txt"), "untracked content 2").unwrap();
            std::fs::create_dir(repo_path.join("untracked_dir")).unwrap();
            std::fs::write(
                repo_path.join("untracked_dir/nested.txt"),
                "nested untracked content",
            )
            .unwrap();

            // Create a .gitignore file
            std::fs::write(repo_path.join(".gitignore"), "ignored.txt\n").unwrap();

            // Create an ignored file
            std::fs::write(repo_path.join("ignored.txt"), "ignored content").unwrap();

            // Get untracked files
            let untracked_files = git_ops.get_untracked_files(repo_path).await.unwrap();

            // Should include untracked files but not ignored ones
            assert!(untracked_files.contains(&PathBuf::from("untracked1.txt")));
            assert!(untracked_files.contains(&PathBuf::from("untracked2.txt")));
            // Git reports the directory, not individual files within it
            assert!(untracked_files.contains(&PathBuf::from("untracked_dir/")));
            assert!(untracked_files.contains(&PathBuf::from(".gitignore")));
            assert!(!untracked_files.contains(&PathBuf::from("ignored.txt")));
            assert!(!untracked_files.contains(&PathBuf::from("tracked.txt")));
        }

        #[tokio::test]
        async fn test_mock_create_branch_from_commit() {
            use super::tests::MockGitOperations;
            let mock = MockGitOperations::new();

            // Test create_branch_from_commit
            let result = mock
                .create_branch_from_commit(
                    Path::new("/test"),
                    "feature-branch",
                    "abc123def456789012345678901234567890abcd",
                )
                .await;
            assert!(result.is_ok());

            // Verify the call was recorded
            let calls = mock.get_create_branch_from_commit_calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, "/test");
            assert_eq!(calls[0].1, "feature-branch");
            assert_eq!(calls[0].2, "abc123def456789012345678901234567890abcd");
        }
    }
}
