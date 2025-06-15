use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

#[async_trait]
pub trait FileSystemOperations: Send + Sync {
    async fn create_dir(&self, path: &Path) -> Result<()>;
    async fn copy_dir(&self, from: &Path, to: &Path) -> Result<()>;
    async fn write_file(&self, path: &Path, content: &str) -> Result<()>;
    async fn read_file(&self, path: &Path) -> Result<String>;
    async fn exists(&self, path: &Path) -> Result<bool>;
    async fn remove_dir(&self, path: &Path) -> Result<()>;
    async fn remove_file(&self, path: &Path) -> Result<()>;
    async fn copy_file(&self, from: &Path, to: &Path) -> Result<()>;
    async fn read_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>>;
}

pub struct DefaultFileSystem;

#[async_trait]
impl FileSystemOperations for DefaultFileSystem {
    async fn create_dir(&self, path: &Path) -> Result<()> {
        tokio::fs::create_dir_all(path).await?;
        Ok(())
    }

    async fn copy_dir(&self, from: &Path, to: &Path) -> Result<()> {
        self.create_dir(to).await?;

        let mut entries = tokio::fs::read_dir(from).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let relative_path = path.strip_prefix(from)?;
            let dst_path = to.join(relative_path);

            if entry.file_type().await?.is_dir() {
                self.copy_dir(&path, &dst_path).await?;
            } else {
                if let Some(parent) = dst_path.parent() {
                    self.create_dir(parent).await?;
                }
                self.copy_file(&path, &dst_path).await?;
            }
        }

        Ok(())
    }

    async fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            self.create_dir(parent).await?;
        }
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    async fn read_file(&self, path: &Path) -> Result<String> {
        let content = tokio::fs::read_to_string(path).await?;
        Ok(content)
    }

    async fn exists(&self, path: &Path) -> Result<bool> {
        Ok(tokio::fs::try_exists(path).await.unwrap_or(false))
    }

    async fn remove_dir(&self, path: &Path) -> Result<()> {
        tokio::fs::remove_dir_all(path).await?;
        Ok(())
    }

    async fn remove_file(&self, path: &Path) -> Result<()> {
        tokio::fs::remove_file(path).await?;
        Ok(())
    }

    async fn copy_file(&self, from: &Path, to: &Path) -> Result<()> {
        tokio::fs::copy(from, to).await?;
        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>> {
        let mut entries = tokio::fs::read_dir(path).await?;
        let mut paths = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            paths.push(entry.path());
        }

        Ok(paths)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub struct MockFileSystem {
        files: Arc<Mutex<HashMap<String, String>>>,
        dirs: Arc<Mutex<Vec<String>>>,
    }

    impl MockFileSystem {
        pub fn new() -> Self {
            Self {
                files: Arc::new(Mutex::new(HashMap::new())),
                dirs: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn with_file(self, path: &str, content: &str) -> Self {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            self
        }

        pub fn with_dir(self, path: &str) -> Self {
            self.dirs.lock().unwrap().push(path.to_string());
            self
        }

        #[allow(dead_code)]
        pub fn get_files(&self) -> HashMap<String, String> {
            self.files.lock().unwrap().clone()
        }

        #[allow(dead_code)]
        pub fn get_dirs(&self) -> Vec<String> {
            self.dirs.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FileSystemOperations for MockFileSystem {
        async fn create_dir(&self, path: &Path) -> Result<()> {
            let path_str = path.to_string_lossy().to_string();
            self.dirs.lock().unwrap().push(path_str);
            Ok(())
        }

        async fn copy_dir(&self, from: &Path, to: &Path) -> Result<()> {
            let from_str = from.to_string_lossy().to_string();
            let to_str = to.to_string_lossy().to_string();

            let files = self.files.lock().unwrap();
            let mut new_files = HashMap::new();

            for (path, content) in files.iter() {
                if path.starts_with(&from_str) {
                    let relative = path.strip_prefix(&from_str).unwrap();
                    let new_path = format!("{}{}", to_str, relative);
                    new_files.insert(new_path, content.clone());
                }
            }

            drop(files);
            self.files.lock().unwrap().extend(new_files);
            self.dirs.lock().unwrap().push(to_str);
            Ok(())
        }

        async fn write_file(&self, path: &Path, content: &str) -> Result<()> {
            let path_str = path.to_string_lossy().to_string();
            self.files
                .lock()
                .unwrap()
                .insert(path_str, content.to_string());
            Ok(())
        }

        async fn read_file(&self, path: &Path) -> Result<String> {
            let path_str = path.to_string_lossy().to_string();
            self.files
                .lock()
                .unwrap()
                .get(&path_str)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("File not found: {}", path_str))
        }

        async fn exists(&self, path: &Path) -> Result<bool> {
            let path_str = path.to_string_lossy().to_string();
            let files = self.files.lock().unwrap();
            let dirs = self.dirs.lock().unwrap();
            Ok(files.contains_key(&path_str) || dirs.contains(&path_str))
        }

        async fn remove_dir(&self, path: &Path) -> Result<()> {
            let path_str = path.to_string_lossy().to_string();
            self.dirs
                .lock()
                .unwrap()
                .retain(|p| !p.starts_with(&path_str));
            self.files
                .lock()
                .unwrap()
                .retain(|p, _| !p.starts_with(&path_str));
            Ok(())
        }

        async fn remove_file(&self, path: &Path) -> Result<()> {
            let path_str = path.to_string_lossy().to_string();
            self.files
                .lock()
                .unwrap()
                .remove(&path_str)
                .ok_or_else(|| anyhow::anyhow!("File not found: {}", path_str))?;
            Ok(())
        }

        async fn copy_file(&self, from: &Path, to: &Path) -> Result<()> {
            let from_str = from.to_string_lossy().to_string();
            let to_str = to.to_string_lossy().to_string();

            let content = self
                .files
                .lock()
                .unwrap()
                .get(&from_str)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Source file not found: {}", from_str))?;

            self.files.lock().unwrap().insert(to_str, content);
            Ok(())
        }

        async fn read_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>> {
            let path_str = path.to_string_lossy().to_string();
            let files = self.files.lock().unwrap();
            let dirs = self.dirs.lock().unwrap();

            let mut entries = Vec::new();

            for file_path in files.keys() {
                if file_path.starts_with(&path_str) && file_path != &path_str {
                    let relative = file_path
                        .strip_prefix(&path_str)
                        .unwrap()
                        .trim_start_matches('/');
                    if !relative.contains('/') {
                        entries.push(std::path::PathBuf::from(file_path));
                    }
                }
            }

            for dir_path in dirs.iter() {
                if dir_path.starts_with(&path_str) && dir_path != &path_str {
                    let relative = dir_path
                        .strip_prefix(&path_str)
                        .unwrap()
                        .trim_start_matches('/');
                    if !relative.contains('/') {
                        entries.push(std::path::PathBuf::from(dir_path));
                    }
                }
            }

            Ok(entries)
        }
    }
}

#[cfg(test)]
#[path = "file_system_tests.rs"]
mod file_system_tests;
