use anyhow::Result;
use std::path::Path;

/// Creates a directory at the specified path, including all parent directories.
pub async fn create_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    Ok(())
}

/// Recursively copies a directory from source to destination.
pub fn copy_dir<'a>(
    from: &'a Path,
    to: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        create_dir(to).await?;

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
                    create_dir(parent).await?;
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
                copy_dir(&path, &dst_path).await?;
            } else {
                if let Some(parent) = dst_path.parent() {
                    create_dir(parent).await?;
                }
                copy_file(&path, &dst_path).await?;
            }
        }

        Ok(())
    })
}

/// Writes content to a file, creating parent directories if needed.
pub async fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_dir(parent).await?;
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

/// Reads the contents of a file as a string.
pub async fn read_file(path: &Path) -> Result<String> {
    let content = tokio::fs::read_to_string(path).await?;
    Ok(content)
}

/// Checks if a path exists (file or directory).
pub async fn exists(path: &Path) -> Result<bool> {
    Ok(tokio::fs::try_exists(path).await.unwrap_or(false))
}

/// Removes a directory and all its contents recursively.
pub async fn remove_dir(path: &Path) -> Result<()> {
    tokio::fs::remove_dir_all(path).await?;
    Ok(())
}

/// Removes a file.
pub async fn remove_file(path: &Path) -> Result<()> {
    tokio::fs::remove_file(path).await?;
    Ok(())
}

/// Copies a file from source to destination.
pub async fn copy_file(from: &Path, to: &Path) -> Result<()> {
    tokio::fs::copy(from, to).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_and_read_file() {
        let temp_dir = TempDir::new().unwrap();

        let file_path = temp_dir.path().join("test.txt");
        let content = "Hello, world!";

        // Write file
        write_file(&file_path, content).await.unwrap();

        // Read file
        let read_content = read_file(&file_path).await.unwrap();
        assert_eq!(read_content, content);

        // Check exists
        assert!(exists(&file_path).await.unwrap());
    }

    #[tokio::test]
    async fn test_create_dir() {
        let temp_dir = TempDir::new().unwrap();

        let dir_path = temp_dir.path().join("test_dir");

        // Create directory
        create_dir(&dir_path).await.unwrap();

        // Check exists
        assert!(exists(&dir_path).await.unwrap());
    }

    #[tokio::test]
    async fn test_copy_file() {
        let temp_dir = TempDir::new().unwrap();

        let source_path = temp_dir.path().join("source.txt");
        let dest_path = temp_dir.path().join("dest.txt");
        let content = "Test content";

        // Create source file
        write_file(&source_path, content).await.unwrap();

        // Copy file
        copy_file(&source_path, &dest_path).await.unwrap();

        // Verify both files exist and have same content
        assert!(exists(&source_path).await.unwrap());
        assert!(exists(&dest_path).await.unwrap());

        let dest_content = read_file(&dest_path).await.unwrap();
        assert_eq!(dest_content, content);
    }

    #[tokio::test]
    async fn test_copy_dir() {
        let temp_dir = TempDir::new().unwrap();

        let source_dir = temp_dir.path().join("source_dir");
        let dest_dir = temp_dir.path().join("dest_dir");

        // Create source directory structure
        create_dir(&source_dir).await.unwrap();
        write_file(&source_dir.join("file1.txt"), "content1")
            .await
            .unwrap();
        create_dir(&source_dir.join("subdir")).await.unwrap();
        write_file(&source_dir.join("subdir").join("file2.txt"), "content2")
            .await
            .unwrap();

        // Copy directory
        copy_dir(&source_dir, &dest_dir).await.unwrap();

        // Verify structure
        assert!(exists(&dest_dir).await.unwrap());
        assert!(exists(&dest_dir.join("file1.txt")).await.unwrap());
        assert!(exists(&dest_dir.join("subdir")).await.unwrap());
        assert!(
            exists(&dest_dir.join("subdir").join("file2.txt"))
                .await
                .unwrap()
        );

        // Verify content
        let content1 = read_file(&dest_dir.join("file1.txt")).await.unwrap();
        assert_eq!(content1, "content1");

        let content2 = read_file(&dest_dir.join("subdir").join("file2.txt"))
            .await
            .unwrap();
        assert_eq!(content2, "content2");
    }

    #[tokio::test]
    async fn test_copy_dir_with_symlinks() {
        let temp_dir = TempDir::new().unwrap();

        let source_dir = temp_dir.path().join("source_dir");
        let dest_dir = temp_dir.path().join("dest_dir");
        let target_dir = temp_dir.path().join("target_dir");

        // Create source directory structure
        create_dir(&source_dir).await.unwrap();
        write_file(&source_dir.join("regular_file.txt"), "regular content")
            .await
            .unwrap();

        // Create a target directory and file that symlinks will point to
        create_dir(&target_dir).await.unwrap();
        write_file(&target_dir.join("target_file.txt"), "target content")
            .await
            .unwrap();
        create_dir(&target_dir.join("target_subdir")).await.unwrap();
        write_file(
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
        copy_dir(&source_dir, &dest_dir).await.unwrap();

        // Verify regular file was copied
        assert!(exists(&dest_dir.join("regular_file.txt")).await.unwrap());
        let content = read_file(&dest_dir.join("regular_file.txt")).await.unwrap();
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
    async fn test_remove_dir() {
        let temp_dir = TempDir::new().unwrap();

        let test_dir = temp_dir.path().join("test_dir");

        // Create directory with content
        create_dir(&test_dir).await.unwrap();
        write_file(&test_dir.join("file.txt"), "content")
            .await
            .unwrap();

        // Verify it exists
        assert!(exists(&test_dir).await.unwrap());

        // Remove directory
        remove_dir(&test_dir).await.unwrap();

        // Verify it's gone
        assert!(!exists(&test_dir).await.unwrap());
    }

    #[tokio::test]
    async fn test_remove_file() {
        let temp_dir = TempDir::new().unwrap();

        let file_path = temp_dir.path().join("test.txt");
        let content = "Test content";

        // Create file
        write_file(&file_path, content).await.unwrap();

        // Verify it exists
        assert!(exists(&file_path).await.unwrap());

        // Remove file
        remove_file(&file_path).await.unwrap();

        // Verify it's gone
        assert!(!exists(&file_path).await.unwrap());

        // Try to remove non-existent file - should error
        let result = remove_file(&file_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_write_and_read_safety() {
        use tokio::task;

        let temp_dir = TempDir::new().unwrap();
        let file_path = Arc::new(temp_dir.path().join("concurrent_test.json"));

        // Create initial file with valid JSON
        let initial_content = r#"{"tasks": []}"#;
        write_file(&file_path, initial_content).await.unwrap();

        // Spawn multiple concurrent writers and readers
        let mut handles = vec![];

        // Writers - continuously update the file with valid JSON
        for i in 0..5 {
            let path_clone = file_path.clone();
            handles.push(task::spawn(async move {
                for j in 0..20 {
                    let content = format!(r#"{{"task_id": {}, "iteration": {}}}"#, i, j);
                    write_file(&path_clone, &content).await.unwrap();
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
            }));
        }

        // Readers - continuously read and parse the file
        for _ in 0..5 {
            let path_clone = file_path.clone();
            handles.push(task::spawn(async move {
                for _ in 0..50 {
                    let content = read_file(&path_clone).await.unwrap();
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
