use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

/// Trait for abstracting file system operations.
/// This trait enables dependency injection and testability.
#[async_trait]
pub trait FileSystemOperations: Send + Sync {
    /// Creates a directory at the specified path, including all parent directories.
    async fn create_dir(&self, path: &Path) -> Result<()>;

    /// Recursively copies a directory from source to destination.
    async fn copy_dir(&self, from: &Path, to: &Path) -> Result<()>;

    /// Writes content to a file, creating parent directories if needed.
    async fn write_file(&self, path: &Path, content: &str) -> Result<()>;

    /// Reads the contents of a file as a string.
    async fn read_file(&self, path: &Path) -> Result<String>;

    /// Checks if a path exists (file or directory).
    async fn exists(&self, path: &Path) -> Result<bool>;

    /// Removes a directory and all its contents recursively.
    async fn remove_dir(&self, path: &Path) -> Result<()>;

    /// Removes a file.
    async fn remove_file(&self, path: &Path) -> Result<()>;

    /// Copies a file from source to destination.
    async fn copy_file(&self, from: &Path, to: &Path) -> Result<()>;

    /// Lists all entries in a directory.
    async fn read_dir(&self, path: &Path) -> Result<Vec<std::path::PathBuf>>;
}

/// Default implementation of FileSystemOperations using tokio::fs.
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
mod tests {
    use super::*;
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
        assert!(
            fs.exists(&dest_dir.join("subdir").join("file2.txt"))
                .await
                .unwrap()
        );

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
    async fn test_file_system_operations_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<DefaultFileSystem>();
        assert_send_sync::<Arc<dyn FileSystemOperations>>();
    }
}
