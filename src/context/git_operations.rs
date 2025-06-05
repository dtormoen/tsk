use async_trait::async_trait;
use std::path::Path;
use std::process::Output;

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

impl DefaultGitOperations {
    async fn execute_git_command(&self, args: &[&str]) -> Result<Output, String> {
        tokio::process::Command::new("git")
            .args(args)
            .output()
            .await
            .map_err(|e| format!("Failed to execute git command: {}", e))
    }
}

#[async_trait]
impl GitOperations for DefaultGitOperations {
    async fn is_git_repository(&self) -> Result<bool, String> {
        let output = self
            .execute_git_command(&["rev-parse", "--git-dir"])
            .await?;
        Ok(output.status.success())
    }

    async fn create_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "checkout", "-b", branch_name])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to create branch: {}", stderr));
        }

        Ok(())
    }

    async fn get_status(&self, repo_path: &Path) -> Result<String, String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "status", "--porcelain"])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to check git status: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn add_all(&self, repo_path: &Path) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "add", "-A"])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to add changes: {}", stderr));
        }

        Ok(())
    }

    async fn commit(&self, repo_path: &Path, message: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "commit", "-m", message])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to commit changes: {}", stderr));
        }

        Ok(())
    }

    async fn add_remote(
        &self,
        repo_path: &Path,
        remote_name: &str,
        url: &str,
    ) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "remote", "add", remote_name, url])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("already exists") {
                return Err(format!("Failed to add remote: {}", stderr));
            }
        }

        Ok(())
    }

    async fn fetch_branch(
        &self,
        repo_path: &Path,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&[
                "-C",
                repo_path_str,
                "fetch",
                remote_name,
                &format!("{}:{}", branch_name, branch_name),
            ])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to fetch changes: {}", stderr));
        }

        Ok(())
    }

    async fn remove_remote(&self, repo_path: &Path, remote_name: &str) -> Result<(), String> {
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "Invalid repo path".to_string())?;

        let output = self
            .execute_git_command(&["-C", repo_path_str, "remote", "remove", remote_name])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to remove temporary remote: {}", stderr));
        }

        Ok(())
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
