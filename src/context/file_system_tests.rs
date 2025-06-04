#[cfg(test)]
mod tests {
    use crate::context::file_system::tests::MockFileSystem;
    use crate::context::file_system::*;
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
    async fn test_file_system_operations_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<DefaultFileSystem>();
        assert_send_sync::<MockFileSystem>();
        assert_send_sync::<Arc<dyn FileSystemOperations>>();
    }
}
