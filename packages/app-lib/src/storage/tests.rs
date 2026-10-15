use super::*;

#[tokio::test]
async fn scan_reports_completed_areas_and_cache_retains_stale_results_for_display()
 {
    let temp = tempfile::tempdir().unwrap();
    let directories = DirectoryInfo {
        settings_dir: temp.path().into(),
        config_dir: temp.path().into(),
        app_identifier: "test".into(),
    };
    fs::create_dir_all(directories.instances_dir())
        .await
        .unwrap();
    fs::write(directories.instances_dir().join("test.jar"), b"game")
        .await
        .unwrap();
    let events = std::sync::Mutex::new(Vec::new());
    let mut tree = scan_with_directories_progress(
        &directories,
        &temp.path().join("backups"),
        vec![],
        &|event| events.lock().unwrap().push(event),
    )
    .await
    .unwrap();
    let events = events.into_inner().unwrap();
    let steps: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StorageScanEvent::Progress {
                completed, total, ..
            } => Some((*completed, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(steps[0], (0, 16));
    assert_eq!(*steps.last().unwrap(), (16, 16));
    assert!(
        steps
            .windows(2)
            .all(|pair| pair[1].0 >= pair[0].0 && pair[1].1 == pair[0].1)
    );
    assert!(snapshot_is_fresh(&tree));
    tree.scanned_at = Utc::now() - chrono::Duration::hours(1);
    assert!(!snapshot_is_fresh(&tree));
    let snapshot = StorageCacheFile {
        version: STORAGE_CACHE_VERSION,
        roots: "current-roots".into(),
        tree: tree.clone(),
    };
    let bytes = serde_cbor::to_vec(&snapshot).unwrap();
    assert_eq!(
        decode_snapshot(&bytes, "current-roots")
            .unwrap()
            .total
            .actual,
        tree.total.actual
    );
    assert!(decode_snapshot(&bytes, "different-roots").is_none());
    assert!(decode_snapshot(b"invalid cache", "current-roots").is_none());
    let old = StorageCacheFile {
        version: STORAGE_CACHE_VERSION - 1,
        ..snapshot
    };
    assert!(
        decode_snapshot(&serde_cbor::to_vec(&old).unwrap(), "current-roots")
            .is_none()
    );
    tree.scanned_at = Utc::now() + chrono::Duration::hours(1);
    assert!(!snapshot_is_fresh(&tree));
}

#[tokio::test]
async fn nested_roots_and_external_backup_are_counted_once() {
    let temp = tempfile::tempdir().unwrap();
    let directories = DirectoryInfo {
        settings_dir: temp.path().join("settings"),
        config_dir: temp.path().join("settings/data"),
        app_identifier: "test".into(),
    };
    let backup = temp.path().join("backups");
    for (path, size) in [
        (
            directories.config_dir.join("profiles/example/mods/a.jar"),
            11,
        ),
        (directories.config_dir.join("caches/content-store/hash"), 13),
        (
            directories
                .config_dir
                .join("servers/server/world/region/a.mca"),
            17,
        ),
        (
            directories.config_dir.join("meta/assets/objects/aa/hash"),
            19,
        ),
        (directories.settings_dir.join("release/app.db"), 23),
        (backup.join("objects/aa/blob"), 29),
        (directories.settings_dir.join("unclassified.txt"), 31),
    ] {
        fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        fs::write(path, vec![1; size]).await.unwrap();
    }
    let tree = scan_with_directories(&directories, &backup, vec![])
        .await
        .unwrap();
    assert_eq!(tree.total.actual, 143);
    assert!(tree.warnings.is_empty());
    assert_eq!(
        tree.categories
            .iter()
            .find(|node| node.node_type == StorageNodeType::Backups)
            .unwrap()
            .size
            .actual,
        29
    );
    assert_eq!(tree.root_other.unwrap().size.actual, 31);
}

#[tokio::test]
async fn backup_inside_another_category_has_exclusive_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let directories = DirectoryInfo {
        settings_dir: temp.path().into(),
        config_dir: temp.path().into(),
        app_identifier: "test".into(),
    };
    let backup = directories.caches_dir().join("backups");
    fs::create_dir_all(&backup).await.unwrap();
    fs::write(backup.join("blob"), [0; 32]).await.unwrap();
    let tree = scan_with_directories(&directories, &backup, vec![])
        .await
        .unwrap();
    assert_eq!(tree.total.actual, 32);
    assert_eq!(tree.categories.len(), 1);
    assert_eq!(tree.categories[0].node_type, StorageNodeType::Backups);
}

#[tokio::test]
async fn a_link_at_the_scan_root_never_counts_as_owned_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("link");
    fs::create_dir_all(&target).await.unwrap();
    fs::write(target.join("data"), [0; 24]).await.unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    junction::create(&target, &link).unwrap();
    let node = scan(&link, 2, &HashSet::new(), &mut vec![]).await.unwrap();
    assert_eq!(
        node.size,
        StorageSize {
            actual: 0,
            symlink: 24
        }
    );
}

#[tokio::test]
async fn scan_errors_do_not_discard_readable_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("files");
    fs::create_dir_all(&path).await.unwrap();
    fs::write(path.join("valid"), [0; 7]).await.unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        temp.path().join("missing"),
        path.join("broken"),
    )
    .unwrap();
    let mut warnings = vec![];
    let node = scan(&path, 1, &HashSet::new(), &mut warnings)
        .await
        .unwrap();
    assert_eq!(node.size.actual, 7);
    #[cfg(unix)]
    assert_eq!(warnings.len(), 1);
}

#[tokio::test]
async fn database_category_has_release_and_beta_as_direct_children() {
    let temp = tempfile::tempdir().unwrap();
    let directories = DirectoryInfo {
        settings_dir: temp.path().into(),
        config_dir: temp.path().into(),
        app_identifier: "test".into(),
    };
    for channel in ["release", "beta"] {
        fs::create_dir_all(temp.path().join(channel)).await.unwrap();
        fs::write(temp.path().join(channel).join("app.db"), [0; 9])
            .await
            .unwrap();
    }
    let tree = scan_with_directories(
        &directories,
        &temp.path().join("backups"),
        vec![],
    )
    .await
    .unwrap();
    let database = tree
        .categories
        .iter()
        .find(|node| node.node_type == StorageNodeType::Database)
        .unwrap();
    assert!(database.name.is_none());
    let names: HashSet<_> = database
        .children
        .as_ref()
        .unwrap()
        .iter()
        .map(|node| node.name.as_deref().unwrap())
        .collect();
    assert_eq!(names, HashSet::from(["release", "beta"]));
    assert_eq!(database.size.actual, 18);
}
