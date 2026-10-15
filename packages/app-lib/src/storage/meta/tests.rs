use super::*;
use serde_json::json;

fn directories(root: &Path) -> DirectoryInfo {
    DirectoryInfo {
        settings_dir: root.into(),
        config_dir: root.into(),
        app_identifier: "test".into(),
    }
}

async fn fixture(root: &Path, id: &str, hash: &str) {
    let version = json!({
        "id": id, "assetIndex": {"id": id, "sha1": "", "size": 0, "totalSize": 0, "url": ""},
        "assets": "legacy", "downloads": {}, "libraries": [{"name": "example:shared:1.0", "natives": {"windows": "natives-windows-${arch}"}}],
        "mainClass": "Main", "minimumLauncherVersion": 0,
        "releaseTime": "2020-01-01T00:00:00Z", "time": "2020-01-01T00:00:00Z", "type": "release"
    });
    let version_path = root.join("versions").join(id);
    fs::create_dir_all(&version_path).await.unwrap();
    fs::write(
        version_path.join(format!("{id}.json")),
        serde_json::to_vec(&version).unwrap(),
    )
    .await
    .unwrap();
    fs::create_dir_all(root.join("assets/indexes"))
        .await
        .unwrap();
    fs::write(
        root.join("assets/indexes").join(format!("{id}.json")),
        serde_json::to_vec(
            &json!({"objects": {"sound/test.ogg": {"hash": hash, "size": 3}}}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_assets_and_legacy_paths_are_retained_across_versions() {
    let temp = tempfile::tempdir().unwrap();
    let hash = "a".repeat(40);
    fixture(temp.path(), "first", &hash).await;
    fixture(temp.path(), "second", &hash).await;
    let mut refs = References::default();
    version_references(temp.path(), "first", &mut refs)
        .await
        .unwrap();
    version_references(temp.path(), "second", &mut refs)
        .await
        .unwrap();
    assert_eq!(refs.used(&format!("assets/objects/aa/{hash}")), Some(true));
    assert_eq!(
        refs.used("libraries/example/shared/1.0/shared-1.0.jar"),
        Some(true)
    );
    assert_eq!(refs.used("resources/sound/test.ogg"), Some(true));
    assert_eq!(
        refs.used(&format!("assets/objects/bb/{}", "b".repeat(40))),
        Some(false)
    );
    assert_eq!(
        refs.used(
            "libraries/example/shared/1.0/shared-1.0-natives-windows-32.jar"
        ),
        Some(true)
    );
    assert_eq!(
        refs.used(
            "libraries/example/shared/1.0/shared-1.0-natives-windows-64.jar"
        ),
        Some(true)
    );
    refs.blocked = true;
    assert_eq!(refs.used("assets/objects/bb/orphan"), None);
}

#[tokio::test]
async fn malformed_asset_hash_and_path_escape_fail_reference_analysis() {
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path(), "test", "bad").await;
    assert!(
        version_references(temp.path(), "test", &mut References::default())
            .await
            .is_err()
    );
    for path in [
        "../outside",
        "libraries/../../outside",
        "C:/outside",
        "libraries\\outside",
    ] {
        assert!(References::default().keep(path.into()).is_err());
    }
}

#[tokio::test]
async fn cleanup_rechecks_references_and_file_changes() {
    let temp = tempfile::tempdir().unwrap();
    let directories = directories(temp.path());
    let root_path = directories.metadata_dir();
    fs::create_dir_all(root_path.join("libraries/example/test/1.0"))
        .await
        .unwrap();
    for name in ["unused.jar", "changed.jar", "newly-used.jar"] {
        fs::write(
            root_path.join("libraries/example/test/1.0").join(name),
            [0; 3],
        )
        .await
        .unwrap();
    }
    let root = opened_root(&directories).unwrap();
    let mut groups = BTreeMap::new();
    let mut files = vec![];
    collect(
        &root,
        "",
        &References::default(),
        &mut groups,
        &mut files,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(files.len(), 3);
    fs::write(
        root_path.join("libraries/example/test/1.0/changed.jar"),
        [0; 9],
    )
    .await
    .unwrap();
    let mut refs = References::default();
    refs.keep("libraries/example/test/1.0/newly-used.jar".into())
        .unwrap();
    let result = remove_candidates(
        &directories,
        files,
        &HashSet::from(["libraries".into()]),
        &refs,
    )
    .unwrap();
    assert_eq!(result.removed_files, 1);
    assert_eq!(result.removed_bytes, 3);
    assert_eq!(result.skipped_files, 2);
    assert!(result.failures.is_empty());
    assert!(
        !root_path
            .join("libraries/example/test/1.0/unused.jar")
            .exists()
    );
    assert!(
        root_path
            .join("libraries/example/test/1.0/changed.jar")
            .exists()
    );
    assert!(
        root_path
            .join("libraries/example/test/1.0/newly-used.jar")
            .exists()
    );
}

#[tokio::test]
async fn unconfirmed_categories_and_processor_libraries_are_never_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let directories = directories(temp.path());
    for path in [
        "java_versions/java/bin/java.exe",
        "game-locales/v1/snapshots/a.json",
        "unknown/data",
        "libraries/example.jar",
    ] {
        let path = directories.metadata_dir().join(path);
        fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        fs::write(path, [0; 8]).await.unwrap();
    }
    let mut refs = References::default();
    refs.paths.insert("libraries/*".into());
    let mut groups = BTreeMap::new();
    let mut files = vec![];
    collect(
        &opened_root(&directories).unwrap(),
        "",
        &refs,
        &mut groups,
        &mut files,
        &mut vec![],
    )
    .unwrap();
    assert!(files.is_empty());
    assert_eq!(groups.values().map(|group| group.unknown).sum::<u64>(), 32);
}

#[tokio::test]
async fn cleanup_cannot_follow_a_replaced_parent_directory() {
    let temp = tempfile::tempdir().unwrap();
    let directories = directories(temp.path());
    let parent = directories
        .metadata_dir()
        .join("libraries/example/test/1.0");
    fs::create_dir_all(&parent).await.unwrap();
    fs::write(parent.join("unused.jar"), [0; 3]).await.unwrap();
    let mut files = vec![];
    collect(
        &opened_root(&directories).unwrap(),
        "",
        &References::default(),
        &mut BTreeMap::new(),
        &mut files,
        &mut vec![],
    )
    .unwrap();
    let original = temp.path().join("original");
    fs::rename(&parent, &original).await.unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).await.unwrap();
    fs::write(outside.join("unused.jar"), [0; 3]).await.unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &parent).unwrap();
    #[cfg(windows)]
    junction::create(&outside, &parent).unwrap();
    let result = remove_candidates(
        &directories,
        files,
        &HashSet::from(["libraries".into()]),
        &References::default(),
    )
    .unwrap();
    assert_eq!(result.removed_files, 0);
    assert_eq!(result.failures.len(), 1);
    assert!(outside.join("unused.jar").exists());
    assert!(original.join("unused.jar").exists());
}

#[test]
fn unknown_files_inside_recognized_folders_are_kept() {
    let refs = References::default();
    for path in [
        "versions/old/user-notes.txt",
        "libraries/custom/file.txt",
        "assets/custom/data",
        "assets/objects/aa/not-a-hash",
        "log_configs/user.txt",
    ] {
        assert_eq!(refs.used(path), None, "{path}");
    }
    assert_eq!(refs.used("versions/old/old.jar"), Some(false));
}

#[tokio::test]
async fn channel_references_union_versions_and_detect_pending_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let directories = directories(temp.path());
    let mut refs = References::default();
    for version in ["first", "second"] {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO instances (id, path, install_stage, launcher_feature_version, name, created, modified) VALUES ('instance', 'example', 'installed', 'none', 'Example', 0, 0)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO instance_content_sets (id, instance_id, name, source_kind, status, game_version, loader, created, modified) VALUES ('set', 'instance', 'Default', 'local', 'available', ?, 'vanilla', 0, 0)").bind(version).execute(&pool).await.unwrap();
        channel_references(&pool, &mut refs, &directories)
            .await
            .unwrap();
        if version == "second" {
            sqlx::query("INSERT INTO install_jobs(id, kind, status, state, created, modified) VALUES ('job', 'install_instance', 'failed', '{\"rollback\":{}}', 0, 0)").execute(&pool).await.unwrap();
            let mut recovering = References::default();
            channel_references(&pool, &mut recovering, &directories)
                .await
                .unwrap();
            assert!(recovering.busy);
            assert_eq!(recovering.used("versions/orphan/orphan.jar"), None);
        }
        pool.close().await;
    }
    assert!(refs.versions.contains("first"));
    assert!(refs.versions.contains("second"));
    assert_eq!(refs.used("versions/first/first.jar"), Some(true));
    assert_eq!(refs.used("versions/second/second.jar"), Some(true));
    assert_eq!(refs.used("versions/orphan/orphan.jar"), Some(false));
}

#[tokio::test]
async fn shared_metadata_leases_exclude_cleanup_until_every_user_finishes() {
    use fs4::tokio::AsyncFileExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("lock");
    let open = || async {
        tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .await
            .unwrap()
    };
    let first = open().await;
    let second = open().await;
    let cleanup = open().await;
    first.lock_shared().unwrap();
    second.lock_shared().unwrap();
    assert!(!cleanup.try_lock_exclusive().unwrap());
    drop(first);
    assert!(!cleanup.try_lock_exclusive().unwrap());
    drop(second);
    assert!(cleanup.try_lock_exclusive().unwrap());
}
