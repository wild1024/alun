//! alun-fs 功能测试
//!
//! 覆盖：LocalFs write/read/delete/exists

#[cfg(test)]
mod tests {
    use alun_fs::LocalFs;
    use std::path::PathBuf;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join("alun_fs_test").join(uuid::Uuid::new_v4().to_string())
    }

    // ──── 创建实例 ───────────────────────────────────

    #[tokio::test]
    async fn test_local_fs_new() {
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        let _fs = LocalFs::new(root.to_str().unwrap());
        assert!(root.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_root_dir() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        assert_eq!(fs.root_dir(), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ──── write / read ───────────────────────────────

    #[tokio::test]
    async fn test_write_and_read() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        let data = b"Hello, Alun FS!";

        let meta = fs.write("test.txt", data).await.unwrap();
        assert_eq!(meta.original_name, "test.txt");

        let read_data = fs.read("test.txt").await.unwrap();
        assert_eq!(read_data, data);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_write_binary_data() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        let data: Vec<u8> = (0..255).collect();

        let meta = fs.write("binary.bin", &data).await.unwrap();
        assert_eq!(meta.size, 255);

        let read_data = fs.read("binary.bin").await.unwrap();
        assert_eq!(read_data, data);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_write_with_name() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        let data = b"image content";

        let meta = fs.write_with_name("photo.jpg", data).await.unwrap();
        assert_eq!(meta.original_name, "photo.jpg");
        assert!(meta.stored_path.contains('/'));

        let read_data = fs.read(&meta.stored_path).await.unwrap();
        assert_eq!(read_data, data);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ──── delete ─────────────────────────────────────

    #[tokio::test]
    async fn test_delete_existing() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        fs.write("del_test.txt", b"data").await.unwrap();
        assert!(fs.exists("del_test.txt").await);

        fs.delete("del_test.txt").await.unwrap();
        assert!(!fs.exists("del_test.txt").await);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        fs.delete("no_file.txt").await.unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    // ──── exists ─────────────────────────────────────

    #[tokio::test]
    async fn test_exists_true() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        fs.write("exist.txt", b"1").await.unwrap();
        assert!(fs.exists("exist.txt").await);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_exists_false() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        assert!(!fs.exists("ghost.txt").await);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ──── read nonexistent ───────────────────────────

    #[tokio::test]
    async fn test_read_nonexistent() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        let result = fs.read("no_file.txt").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("文件不存在"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ──── MIME 类型 ──────────────────────────────────

    #[tokio::test]
    async fn test_content_type_for_images() {
        let root = test_root();
        let fs = LocalFs::new(root.to_str().unwrap());
        let meta = fs.write("img.png", b"fake").await.unwrap();
        assert_eq!(meta.content_type, "image/png");

        let meta = fs.write("img.jpg", b"fake").await.unwrap();
        assert_eq!(meta.content_type, "image/jpeg");

        let _ = std::fs::remove_dir_all(&root);
    }
}