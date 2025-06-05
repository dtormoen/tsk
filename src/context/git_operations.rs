use async_trait::async_trait;
use git2::{Repository, RepositoryOpenFlags};
use std::path::Path;

#[async_trait]
pub trait GitOperations: Send + Sync {
    async fn is_git_repository(&self) -> Result<bool, String>;

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
}

pub struct DefaultGitOperations;

impl DefaultGitOperations {}

#[async_trait]
impl GitOperations for DefaultGitOperations {
    async fn is_git_repository(&self) -> Result<bool, String> {
        let current_dir = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;

        match Repository::open_ext(&current_dir, RepositoryOpenFlags::empty(), &[] as &[&Path]) {
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
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let head = repo
                    .head()
                    .map_err(|e| format!("Failed to get HEAD: {}", e))?;

                let commit = head
                    .peel_to_commit()
                    .map_err(|e| format!("Failed to get commit from HEAD: {}", e))?;

                repo.branch(&branch_name, &commit, false)
                    .map_err(|e| format!("Failed to create branch: {}", e))?;

                repo.set_head(&format!("refs/heads/{}", branch_name))
                    .map_err(|e| format!("Failed to checkout branch: {}", e))?;

                repo.checkout_head(None)
                    .map_err(|e| format!("Failed to update working directory: {}", e))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn get_status(&self, repo_path: &Path) -> Result<String, String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<String, String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let statuses = repo
                    .statuses(None)
                    .map_err(|e| format!("Failed to get repository status: {}", e))?;

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

                        result.push_str(&format!("{} {}\n", status_char, path));
                    }
                }

                Ok(result)
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn add_all(&self, repo_path: &Path) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {}", e))?;

                index
                    .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                    .map_err(|e| format!("Failed to add files to index: {}", e))?;

                index
                    .write()
                    .map_err(|e| format!("Failed to write index: {}", e))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let message = message.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let mut index = repo
                    .index()
                    .map_err(|e| format!("Failed to get repository index: {}", e))?;

                let tree_id = index
                    .write_tree()
                    .map_err(|e| format!("Failed to write tree: {}", e))?;

                let tree = repo
                    .find_tree(tree_id)
                    .map_err(|e| format!("Failed to find tree: {}", e))?;

                let signature = repo
                    .signature()
                    .map_err(|e| format!("Failed to get signature: {}", e))?;

                let parent_commit = match repo.head() {
                    Ok(head) => Some(
                        head.peel_to_commit()
                            .map_err(|e| format!("Failed to get parent commit: {}", e))?,
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
                .map_err(|e| format!("Failed to create commit: {}", e))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
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
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let result = repo.remote(&remote_name, &url);
                match result {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        if e.code() == git2::ErrorCode::Exists {
                            Ok(())
                        } else {
                            Err(format!("Failed to add remote: {}", e))
                        }
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
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
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                let mut remote = repo
                    .find_remote(&remote_name)
                    .map_err(|e| format!("Failed to find remote: {}", e))?;

                let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);

                remote
                    .fetch(&[&refspec], None, None)
                    .map_err(|e| format!("Failed to fetch changes: {}", e))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String> {
        tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            let remote_name = remote_name.to_owned();
            move || -> Result<(), String> {
                let repo = Repository::open(&repo_path)
                    .map_err(|e| format!("Failed to open repository: {}", e))?;

                repo.remote_delete(&remote_name)
                    .map_err(|e| format!("Failed to remove temporary remote: {}", e))?;

                Ok(())
            }
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
}

#[cfg(test)]
pub mod tests {
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
            }
        }

        pub fn set_is_repo_result(&self, result: Result<bool, String>) {
            *self.is_repo_result.lock().unwrap() = result;
        }

        pub fn set_get_status_result(&self, result: Result<String, String>) {
            *self.get_status_result.lock().unwrap() = result;
        }

        #[allow(dead_code)]
        pub fn set_create_branch_result(&self, result: Result<(), String>) {
            *self.create_branch_result.lock().unwrap() = result;
        }

        #[allow(dead_code)]
        pub fn set_commit_result(&self, result: Result<(), String>) {
            *self.commit_result.lock().unwrap() = result;
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
    }

    #[async_trait]
    impl GitOperations for MockGitOperations {
        async fn is_git_repository(&self) -> Result<bool, String> {
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
    }
}

#[cfg(test)]
mod git_operations_tests;
