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
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
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

        pub fn get_files(&self) -> HashMap<String, String> {
            self.files.lock().unwrap().clone()
        }

        pub fn get_dirs(&self) -> Vec<String> {
            self.dirs.lock().unwrap().clone()
        }

        pub fn set_files(&self, files: HashMap<PathBuf, String>) {
            let mut file_map = self.files.lock().unwrap();
            file_map.clear();
            for (path, content) in files {
                let path_str = path.to_string_lossy().to_string();
                if content == "dir" {
                    self.dirs.lock().unwrap().push(path_str);
                } else {
                    file_map.insert(path_str, content);
                }
            }
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
                    let new_path = format!("{to_str}{relative}");
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

    #[cfg(test)]
    mod integration_tests {
        use super::*;
        use std::path::Path;
        use std::sync::Arc;
        use tempfile::TempDir;

        #[tokio::test]
        async fn test_default_file_system_create_and_read_file() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let file_path = temp_dir.path().join("test.txt");
            let content = "Hello, world!";

            // Write file
            fs.write_file(&file_path, content).await.unwrap();

            // Read file
            let read_content = fs.read_file(&file_path).await.unwrap();
            assert_eq!(read_content, content);

            // Check exists
            assert!(fs.exists(&file_path).await.unwrap());
        }

        #[tokio::test]
        async fn test_default_file_system_create_dir() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let dir_path = temp_dir.path().join("test_dir");

            // Create directory
            fs.create_dir(&dir_path).await.unwrap();

            // Check exists
            assert!(fs.exists(&dir_path).await.unwrap());
        }

        #[tokio::test]
        async fn test_default_file_system_copy_file() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let source_path = temp_dir.path().join("source.txt");
            let dest_path = temp_dir.path().join("dest.txt");
            let content = "Test content";

            // Create source file
            fs.write_file(&source_path, content).await.unwrap();

            // Copy file
            fs.copy_file(&source_path, &dest_path).await.unwrap();

            // Verify both files exist and have same content
            assert!(fs.exists(&source_path).await.unwrap());
            assert!(fs.exists(&dest_path).await.unwrap());

            let dest_content = fs.read_file(&dest_path).await.unwrap();
            assert_eq!(dest_content, content);
        }

        #[tokio::test]
        async fn test_default_file_system_copy_dir() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let source_dir = temp_dir.path().join("source_dir");
            let dest_dir = temp_dir.path().join("dest_dir");

            // Create source directory structure
            fs.create_dir(&source_dir).await.unwrap();
            fs.write_file(&source_dir.join("file1.txt"), "content1")
                .await
                .unwrap();
            fs.create_dir(&source_dir.join("subdir")).await.unwrap();
            fs.write_file(&source_dir.join("subdir").join("file2.txt"), "content2")
                .await
                .unwrap();

            // Copy directory
            fs.copy_dir(&source_dir, &dest_dir).await.unwrap();

            // Verify structure
            assert!(fs.exists(&dest_dir).await.unwrap());
            assert!(fs.exists(&dest_dir.join("file1.txt")).await.unwrap());
            assert!(fs.exists(&dest_dir.join("subdir")).await.unwrap());
            assert!(fs
                .exists(&dest_dir.join("subdir").join("file2.txt"))
                .await
                .unwrap());

            // Verify content
            let content1 = fs.read_file(&dest_dir.join("file1.txt")).await.unwrap();
            assert_eq!(content1, "content1");

            let content2 = fs
                .read_file(&dest_dir.join("subdir").join("file2.txt"))
                .await
                .unwrap();
            assert_eq!(content2, "content2");
        }

        #[tokio::test]
        async fn test_default_file_system_read_dir() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let test_dir = temp_dir.path().join("test_dir");
            fs.create_dir(&test_dir).await.unwrap();

            // Create some files and directories
            fs.write_file(&test_dir.join("file1.txt"), "content1")
                .await
                .unwrap();
            fs.write_file(&test_dir.join("file2.txt"), "content2")
                .await
                .unwrap();
            fs.create_dir(&test_dir.join("subdir")).await.unwrap();

            // Read directory
            let entries = fs.read_dir(&test_dir).await.unwrap();

            // Should have 3 entries
            assert_eq!(entries.len(), 3);

            // Check that all expected paths are present
            let entry_names: Vec<String> = entries
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect();

            assert!(entry_names.contains(&"file1.txt".to_string()));
            assert!(entry_names.contains(&"file2.txt".to_string()));
            assert!(entry_names.contains(&"subdir".to_string()));
        }

        #[tokio::test]
        async fn test_default_file_system_remove_dir() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let test_dir = temp_dir.path().join("test_dir");

            // Create directory with content
            fs.create_dir(&test_dir).await.unwrap();
            fs.write_file(&test_dir.join("file.txt"), "content")
                .await
                .unwrap();

            // Verify it exists
            assert!(fs.exists(&test_dir).await.unwrap());

            // Remove directory
            fs.remove_dir(&test_dir).await.unwrap();

            // Verify it's gone
            assert!(!fs.exists(&test_dir).await.unwrap());
        }

        #[tokio::test]
        async fn test_mock_file_system() {
            let mock_fs = MockFileSystem::new()
                .with_file("/test/file.txt", "test content")
                .with_dir("/test");

            // Test file exists
            assert!(mock_fs.exists(Path::new("/test/file.txt")).await.unwrap());

            // Test dir exists
            assert!(mock_fs.exists(Path::new("/test")).await.unwrap());

            // Test read file
            let content = mock_fs
                .read_file(Path::new("/test/file.txt"))
                .await
                .unwrap();
            assert_eq!(content, "test content");

            // Test write file
            mock_fs
                .write_file(Path::new("/test/new.txt"), "new content")
                .await
                .unwrap();
            let new_content = mock_fs.read_file(Path::new("/test/new.txt")).await.unwrap();
            assert_eq!(new_content, "new content");

            // Test create dir
            mock_fs.create_dir(Path::new("/new_dir")).await.unwrap();
            assert!(mock_fs.exists(Path::new("/new_dir")).await.unwrap());

            // Test copy file
            mock_fs
                .copy_file(Path::new("/test/file.txt"), Path::new("/test/copy.txt"))
                .await
                .unwrap();
            let copied = mock_fs
                .read_file(Path::new("/test/copy.txt"))
                .await
                .unwrap();
            assert_eq!(copied, "test content");

            // Test remove dir
            mock_fs.remove_dir(Path::new("/test")).await.unwrap();
            assert!(!mock_fs.exists(Path::new("/test")).await.unwrap());
            assert!(!mock_fs.exists(Path::new("/test/file.txt")).await.unwrap());
        }

        #[tokio::test]
        async fn test_mock_file_system_copy_dir() {
            let mock_fs = MockFileSystem::new()
                .with_dir("/source")
                .with_file("/source/file1.txt", "content1")
                .with_dir("/source/subdir")
                .with_file("/source/subdir/file2.txt", "content2");

            // Copy directory
            mock_fs
                .copy_dir(Path::new("/source"), Path::new("/dest"))
                .await
                .unwrap();

            // Verify files were copied
            assert!(mock_fs.exists(Path::new("/dest")).await.unwrap());
            assert_eq!(
                mock_fs
                    .read_file(Path::new("/dest/file1.txt"))
                    .await
                    .unwrap(),
                "content1"
            );
            assert_eq!(
                mock_fs
                    .read_file(Path::new("/dest/subdir/file2.txt"))
                    .await
                    .unwrap(),
                "content2"
            );
        }

        #[tokio::test]
        async fn test_mock_file_system_read_dir() {
            let mock_fs = MockFileSystem::new()
                .with_dir("/test")
                .with_file("/test/file1.txt", "content1")
                .with_file("/test/file2.txt", "content2")
                .with_dir("/test/subdir")
                .with_file("/test/subdir/nested.txt", "nested");

            // Read directory
            let entries = mock_fs.read_dir(Path::new("/test")).await.unwrap();

            // Should have 3 entries (2 files + 1 dir at the top level)
            assert_eq!(entries.len(), 3);

            // Check that entries are correct
            let entry_strs: Vec<String> = entries
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();

            assert!(entry_strs.contains(&"/test/file1.txt".to_string()));
            assert!(entry_strs.contains(&"/test/file2.txt".to_string()));
            assert!(entry_strs.contains(&"/test/subdir".to_string()));

            // Should not include nested file
            assert!(!entry_strs.contains(&"/test/subdir/nested.txt".to_string()));
        }

        #[tokio::test]
        async fn test_default_file_system_remove_file() {
            let temp_dir = TempDir::new().unwrap();
            let fs = DefaultFileSystem;

            let file_path = temp_dir.path().join("test.txt");
            let content = "Test content";

            // Create file
            fs.write_file(&file_path, content).await.unwrap();

            // Verify it exists
            assert!(fs.exists(&file_path).await.unwrap());

            // Remove file
            fs.remove_file(&file_path).await.unwrap();

            // Verify it's gone
            assert!(!fs.exists(&file_path).await.unwrap());

            // Try to remove non-existent file - should error
            let result = fs.remove_file(&file_path).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_mock_file_system_remove_file() {
            let mock_fs = MockFileSystem::new().with_file("/test/file.txt", "test content");

            // Verify file exists
            assert!(mock_fs.exists(Path::new("/test/file.txt")).await.unwrap());

            // Remove file
            mock_fs
                .remove_file(Path::new("/test/file.txt"))
                .await
                .unwrap();

            // Verify it's gone
            assert!(!mock_fs.exists(Path::new("/test/file.txt")).await.unwrap());

            // Try to remove non-existent file - should error
            let result = mock_fs.remove_file(Path::new("/test/file.txt")).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("File not found"));
        }

        #[tokio::test]
        async fn test_file_system_operations_are_send_sync() {
            fn assert_send_sync<T: Send + Sync>() {}

            assert_send_sync::<DefaultFileSystem>();
            assert_send_sync::<MockFileSystem>();
            assert_send_sync::<Arc<dyn FileSystemOperations>>();
        }
    }
}
