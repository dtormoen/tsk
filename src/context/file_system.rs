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
    #[cfg(test)]
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

            // Get metadata without following symlinks to check if it's a symlink
            let metadata = tokio::fs::symlink_metadata(&path).await?;

            if metadata.is_symlink() {
                // Preserve symlinks by reading the target and creating a new symlink
                let target = tokio::fs::read_link(&path).await?;
                if let Some(parent) = dst_path.parent() {
                    self.create_dir(parent).await?;
                }
                // Create symlink at destination pointing to the same target
                #[cfg(unix)]
                tokio::fs::symlink(&target, &dst_path).await?;
                #[cfg(windows)]
                {
                    // On Windows, we need to determine if it's a file or directory symlink
                    // Try to get the target metadata to determine the type
                    if let Ok(target_metadata) = tokio::fs::metadata(&path).await {
                        if target_metadata.is_dir() {
                            tokio::fs::symlink_dir(&target, &dst_path).await?;
                        } else {
                            tokio::fs::symlink_file(&target, &dst_path).await?;
                        }
                    } else {
                        // If we can't determine the type, try as a file symlink
                        tokio::fs::symlink_file(&target, &dst_path).await?;
                    }
                }
            } else if metadata.is_dir() {
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

        // Generate unique temporary filename to avoid collisions
        let temp_path = {
            let mut temp = path.to_path_buf();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let pid = std::process::id();
            temp.set_file_name(format!(
                ".{}.{}.{}.tmp",
                path.file_name().unwrap_or_default().to_string_lossy(),
                pid,
                timestamp
            ));
            temp
        };

        tokio::fs::write(&temp_path, content).await?;

        match tokio::fs::rename(&temp_path, path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Err(e.into())
            }
        }
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

    #[cfg(test)]
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
    async fn test_default_file_system_copy_dir_with_symlinks() {
        let temp_dir = TempDir::new().unwrap();
        let fs = DefaultFileSystem;

        let source_dir = temp_dir.path().join("source_dir");
        let dest_dir = temp_dir.path().join("dest_dir");
        let target_dir = temp_dir.path().join("target_dir");

        // Create source directory structure
        fs.create_dir(&source_dir).await.unwrap();
        fs.write_file(&source_dir.join("regular_file.txt"), "regular content")
            .await
            .unwrap();

        // Create a target directory and file that symlinks will point to
        fs.create_dir(&target_dir).await.unwrap();
        fs.write_file(&target_dir.join("target_file.txt"), "target content")
            .await
            .unwrap();
        fs.create_dir(&target_dir.join("target_subdir"))
            .await
            .unwrap();
        fs.write_file(
            &target_dir.join("target_subdir").join("nested.txt"),
            "nested content",
        )
        .await
        .unwrap();

        // Create symlinks in source directory
        #[cfg(unix)]
        {
            // Symlink to a file
            tokio::fs::symlink(
                &target_dir.join("target_file.txt"),
                &source_dir.join("symlink_to_file"),
            )
            .await
            .unwrap();

            // Symlink to a directory
            tokio::fs::symlink(
                &target_dir.join("target_subdir"),
                &source_dir.join("symlink_to_dir"),
            )
            .await
            .unwrap();

            // Relative symlink
            tokio::fs::symlink(
                "../target_dir/target_file.txt",
                &source_dir.join("relative_symlink"),
            )
            .await
            .unwrap();
        }

        #[cfg(windows)]
        {
            // On Windows, we need to specify file vs directory symlinks
            tokio::fs::symlink_file(
                &target_dir.join("target_file.txt"),
                &source_dir.join("symlink_to_file"),
            )
            .await
            .unwrap();

            tokio::fs::symlink_dir(
                &target_dir.join("target_subdir"),
                &source_dir.join("symlink_to_dir"),
            )
            .await
            .unwrap();

            tokio::fs::symlink_file(
                "../target_dir/target_file.txt",
                &source_dir.join("relative_symlink"),
            )
            .await
            .unwrap();
        }

        // Copy directory with symlinks
        fs.copy_dir(&source_dir, &dest_dir).await.unwrap();

        // Verify regular file was copied
        assert!(fs.exists(&dest_dir.join("regular_file.txt")).await.unwrap());
        let content = fs
            .read_file(&dest_dir.join("regular_file.txt"))
            .await
            .unwrap();
        assert_eq!(content, "regular content");

        // Verify symlinks were preserved as symlinks (not dereferenced)
        #[cfg(unix)]
        {
            // Check file symlink
            let symlink_metadata = tokio::fs::symlink_metadata(&dest_dir.join("symlink_to_file"))
                .await
                .unwrap();
            assert!(symlink_metadata.is_symlink());

            // Check directory symlink
            let dir_symlink_metadata =
                tokio::fs::symlink_metadata(&dest_dir.join("symlink_to_dir"))
                    .await
                    .unwrap();
            assert!(dir_symlink_metadata.is_symlink());

            // Check relative symlink
            let rel_symlink_metadata =
                tokio::fs::symlink_metadata(&dest_dir.join("relative_symlink"))
                    .await
                    .unwrap();
            assert!(rel_symlink_metadata.is_symlink());

            // Verify symlink targets are preserved
            let link_target = tokio::fs::read_link(&dest_dir.join("symlink_to_file"))
                .await
                .unwrap();
            assert_eq!(link_target, target_dir.join("target_file.txt"));

            let dir_link_target = tokio::fs::read_link(&dest_dir.join("symlink_to_dir"))
                .await
                .unwrap();
            assert_eq!(dir_link_target, target_dir.join("target_subdir"));

            let rel_link_target = tokio::fs::read_link(&dest_dir.join("relative_symlink"))
                .await
                .unwrap();
            assert_eq!(
                rel_link_target.to_string_lossy(),
                "../target_dir/target_file.txt"
            );
        }

        #[cfg(windows)]
        {
            // On Windows, check that symlinks exist and point to correct targets
            let symlink_metadata = tokio::fs::symlink_metadata(&dest_dir.join("symlink_to_file"))
                .await
                .unwrap();
            assert!(symlink_metadata.is_symlink());

            let dir_symlink_metadata =
                tokio::fs::symlink_metadata(&dest_dir.join("symlink_to_dir"))
                    .await
                    .unwrap();
            assert!(dir_symlink_metadata.is_symlink());
        }
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

    #[tokio::test]
    async fn test_concurrent_write_and_read_safety() {
        use std::sync::Arc;
        use tokio::task;

        let temp_dir = TempDir::new().unwrap();
        let fs = Arc::new(DefaultFileSystem);
        let file_path = Arc::new(temp_dir.path().join("concurrent_test.json"));

        // Create initial file with valid JSON
        let initial_content = r#"{"tasks": []}"#;
        fs.write_file(&file_path, initial_content).await.unwrap();

        // Spawn multiple concurrent writers and readers
        let mut handles = vec![];

        // Writers - continuously update the file with valid JSON
        for i in 0..5 {
            let fs_clone = fs.clone();
            let path_clone = file_path.clone();
            handles.push(task::spawn(async move {
                for j in 0..20 {
                    let content = format!(r#"{{"task_id": {}, "iteration": {}}}"#, i, j);
                    fs_clone.write_file(&path_clone, &content).await.unwrap();
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
            }));
        }

        // Readers - continuously read and parse the file
        for _ in 0..5 {
            let fs_clone = fs.clone();
            let path_clone = file_path.clone();
            handles.push(task::spawn(async move {
                for _ in 0..50 {
                    let content = fs_clone.read_file(&path_clone).await.unwrap();
                    // Should always be valid JSON - never empty or partial
                    assert!(!content.is_empty(), "File should never be empty");
                    // Verify it's valid JSON
                    let _: serde_json::Value = serde_json::from_str(&content)
                        .expect("File should always contain valid JSON");
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
            }));
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }
    }
}
