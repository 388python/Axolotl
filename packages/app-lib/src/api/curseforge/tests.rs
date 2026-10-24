//! CurseForge unit tests.

#[cfg(test)]
fn murmur2(data: &[u8], seed: u32) -> u32 {
    const M: u32 = 0x5bd1e995;
    const R: u32 = 24;
    let mut hash = seed ^ data.len() as u32;
    let mut chunks = data.chunks_exact(4);

    for chunk in &mut chunks {
        let mut value =
            u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        value = value.wrapping_mul(M);
        value ^= value >> R;
        value = value.wrapping_mul(M);
        hash = hash.wrapping_mul(M);
        hash ^= value;
    }

    match chunks.remainder() {
        [a, b, c] => {
            hash ^= (*c as u32) << 16;
            hash ^= (*b as u32) << 8;
            hash ^= *a as u32;
            hash = hash.wrapping_mul(M);
        }
        [a, b] => {
            hash ^= (*b as u32) << 8;
            hash ^= *a as u32;
            hash = hash.wrapping_mul(M);
        }
        [a] => {
            hash ^= *a as u32;
            hash = hash.wrapping_mul(M);
        }
        [] => {}
        _ => unreachable!(),
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^= hash >> 15;
    hash
}

use super::*;
use std::io::Write;

#[test]
fn modpack_manifest_worlds_use_the_save_importer_instead_of_the_mod_file_installer()
 {
    assert_eq!(project_type_for_class(Some(17)), "world");
    assert_eq!(modpack_target_folder("world").unwrap(), "saves");
    assert_eq!(modpack_target_folder("mod").unwrap(), "mods");
    assert_eq!(
        modpack_target_folder("resourcepack").unwrap(),
        "resourcepacks"
    );
    assert!(modpack_target_folder("modpack").is_err());
    assert!(managed_project_type("world").is_err());
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn modpack_manifest_imports_world_archives_and_preserves_existing_saves_on_retry()
 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (state, instance_id) =
        create_stage6_instance("manifest world").await;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(
            "Wrapped world/level.dat",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"world level data").unwrap();
    archive
        .start_file(
            "Wrapped world/region/r.0.0.mca",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"region bytes").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let server =
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/world.zip", server.local_addr().unwrap());
    let response_bytes = bytes.clone();
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = server.accept().await.unwrap();
            let bytes = response_bytes.clone();
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let n = stream.read(&mut request).await.unwrap();
                let head = String::from_utf8_lossy(&request[..n])
                    .starts_with("HEAD ");
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\nConnection: close\r\n\r\n", bytes.len()).as_bytes()).await.unwrap();
                if !head {
                    stream.write_all(&bytes).await.unwrap();
                }
            });
        }
    });
    let project: CurseForgeProject = serde_json::from_value(json!({
        "id": 1733396, "gameId": 432, "name": "Pack world", "slug": "pack-world", "summary": "World bundled in a pack",
        "status": 4, "downloadCount": 0, "isFeatured": false, "primaryCategoryId": 0,
        "classId": 17, "mainFileId": 9206584, "dateCreated": "", "dateModified": "", "dateReleased": "", "isAvailable": true,
    })).unwrap();
    let mut file = stage8_curseforge_file(
        project.id,
        9206584,
        "Pack world.zip",
        bytes.len() as u64,
        vec![CurseForgeFileHash {
            algo: 1,
            value: sha1_smol::Sha1::from(&bytes).hexdigest(),
        }],
        0,
    );
    file.download_url = Some(url);
    let manifest_file = CurseForgeManifestFile {
        project_id: project.id,
        file_id: file.id,
        required: true,
    };
    let installed = tokio::time::timeout(Duration::from_secs(10), retry_modpack_file_install(
        &instance_id, &manifest_file, "world", "1.21.1", Some(6),
        crate::state::instances::ManualDownloadOperationKind::PackInstall,
        None, &file.file_name, CancellationToken::new(), None, None, Some((project.clone(), file.clone())),
    )).await.unwrap();
    server_task.abort();
    let result = installed.0.unwrap_or_else(|| panic!("{}", installed.2));
    assert_eq!(result.installed[0].relative_path, "saves/Pack world");
    let root = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(root.join("saves/Pack world/level.dat")).unwrap(),
        b"world level data"
    );
    assert_eq!(
        std::fs::read(root.join("saves/Pack world/region/r.0.0.mca"))
            .unwrap(),
        b"region bytes"
    );
    std::fs::write(
        root.join("saves/Pack world/level.dat"),
        b"player progress",
    )
    .unwrap();
    install_modpack_world_file(
        &instance_id,
        &project,
        &file,
        None,
        crate::state::instances::ManualDownloadOperationKind::PackUpdate,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(root.join("saves/Pack world/level.dat")).unwrap(),
        b"player progress"
    );
    assert!(
        crate::state::get_instance(&instance_id, &state.pool)
            .await
            .unwrap()
            .is_some()
    );
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[test]
fn content_updates_only_resolve_required_dependencies() {
    use crate::state::instances::ManualDownloadOperationKind;

    let operation = ManualDownloadOperationKind::ContentUpdate;
    assert!(should_resolve_dependency(
        DEPENDENCY_RELATION_REQUIRED,
        operation
    ));
    for relation in [
        DEPENDENCY_RELATION_OPTIONAL,
        DEPENDENCY_RELATION_INCLUDE,
        DEPENDENCY_RELATION_EMBEDDED,
        DEPENDENCY_RELATION_TOOL,
        DEPENDENCY_RELATION_INCOMPATIBLE,
    ] {
        assert!(!should_resolve_dependency(relation, operation));
    }
    assert_eq!(
        dependency_policy(operation),
        DependencyPolicy::RequiredOnly
    );
}

#[test]
fn content_installs_still_resolve_optional_dependencies_for_selection() {
    use crate::state::instances::ManualDownloadOperationKind;

    for operation in [
        ManualDownloadOperationKind::ContentInstall,
        ManualDownloadOperationKind::PackInstall,
        ManualDownloadOperationKind::PackUpdate,
    ] {
        for relation in [
            DEPENDENCY_RELATION_REQUIRED,
            DEPENDENCY_RELATION_OPTIONAL,
            DEPENDENCY_RELATION_INCLUDE,
        ] {
            assert!(should_resolve_dependency(relation, operation));
        }
        assert_eq!(
            dependency_policy(operation),
            DependencyPolicy::IncludeOptional
        );
    }
}

fn write_override_test_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    archive.start_file("manifest.json", options).unwrap();
    archive
        .write_all(
            br#"{"minecraft":{"version":"1.21.1","modLoaders":[]},"files":[],"overrides":"overrides"}"#,
        )
        .unwrap();
    for (name, contents) in entries {
        archive
            .start_file(format!("overrides/{name}"), options)
            .unwrap();
        archive.write_all(contents).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn modpack_metadata_ids_are_deduplicated_before_batching() {
    let mut ids = (1..=120).collect::<Vec<_>>();
    ids.extend([1, 50, 120]);
    let chunks = unique_metadata_chunks(ids);

    assert_eq!(
        chunks.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![50, 50, 20]
    );
    assert_eq!(
        chunks
            .iter()
            .flatten()
            .copied()
            .collect::<HashSet<_>>()
            .len(),
        120
    );
}

#[test]
fn modpack_archives_use_bounded_ranges_only_for_large_files() {
    assert_eq!(
        curseforge_modpack_h2_range_concurrency(128 * 1024 * 1024),
        Some(4)
    );
    assert_eq!(curseforge_modpack_h2_range_concurrency(1024 * 1024), None);
}

#[tokio::test]
async fn staged_digest_proof_is_reused_only_for_the_unchanged_file() {
    let directory = tempfile::tempdir().unwrap();
    let staged = directory.path().join("content.installing.download");
    let other = directory.path().join("other.installing.download");
    tokio::fs::write(&staged, b"verified staged bytes")
        .await
        .unwrap();
    let sha1 = sha1_smol::Sha1::from(b"verified staged bytes").hexdigest();
    let file = stage8_curseforge_file(
        1,
        2,
        "content.jar",
        21,
        vec![CurseForgeFileHash {
            value: sha1.clone(),
            algo: 1,
        }],
        0,
    );
    let download = crate::util::fetch::DownloadResult {
        path: staged.clone(),
        url: "https://staged-proof.invalid/file".into(),
        source: crate::util::fetch::DownloadRouteSource::Official,
        size: 21,
        attempts: 1,
        fallback_count: 0,
        verified_sha1: Some(sha1.clone()),
        verified_sha512: None,
        verified_file:
            crate::util::download::verified_file::VerifiedFile::capture(
                &staged, 21,
            )
            .await,
    };
    assert_eq!(
        download
            .verified_file
            .as_ref()
            .unwrap()
            .matches(&staged, 21)
            .await,
        cfg!(unix)
    );
    let (verified, scans) =
        crate::util::download::verified_file::track_scans(
            verify_installed_curseforge_file(
                &staged,
                &file,
                None,
                Some(&download),
            ),
        )
        .await;
    let verified = verified.unwrap();
    assert_eq!(scans, usize::from(!cfg!(unix)));
    assert_eq!(verified.sha1, sha1);
    assert_eq!(
        verified.pending_completion,
        CurseForgePendingCompletionProof::AuthoritativeSha1
    );
    let serialized = serde_json::to_value(&download).unwrap();
    let restored: crate::util::fetch::DownloadResult =
        serde_json::from_value(serialized).unwrap();
    assert!(restored.verified_file.is_none());
    let (rescanned, scans) =
        crate::util::download::verified_file::track_scans(
            verify_installed_curseforge_file(
                &staged,
                &file,
                None,
                Some(&restored),
            ),
        )
        .await;
    assert!(rescanned.is_ok());
    assert_eq!(scans, 1);
    tokio::fs::write(&other, b"corrupted other bytes")
        .await
        .unwrap();
    assert!(
        verify_installed_curseforge_file(
            &other,
            &file,
            None,
            Some(&download)
        )
        .await
        .is_err()
    );
    tokio::fs::write(&staged, b"corrupted staged file")
        .await
        .unwrap();
    let handle =
        std::fs::File::options().write(true).open(&staged).unwrap();
    handle
        .set_times(std::fs::FileTimes::new().set_modified(
            std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(123456),
        ))
        .unwrap();
    assert!(
        verify_installed_curseforge_file(
            &staged,
            &file,
            None,
            Some(&download)
        )
        .await
        .is_err()
    );
    assert!(
        verify_installed_curseforge_file(
            &staged,
            &file,
            None,
            Some(&restored)
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn staged_digest_proof_never_skips_fingerprint_verification() {
    let directory = tempfile::tempdir().unwrap();
    let staged = directory.path().join("content.installing.download");
    tokio::fs::write(&staged, b"verified staged bytes")
        .await
        .unwrap();
    let file = stage8_curseforge_file(1, 2, "content.jar", 21, vec![], 1);
    let download = crate::util::fetch::DownloadResult {
        path: staged.clone(),
        url: "https://staged-proof.invalid/file".into(),
        source: crate::util::fetch::DownloadRouteSource::Official,
        size: 21,
        attempts: 1,
        fallback_count: 0,
        verified_sha1: Some("trusted-sha1".into()),
        verified_sha512: None,
        verified_file:
            crate::util::download::verified_file::VerifiedFile::capture(
                &staged, 21,
            )
            .await,
    };
    let result = verify_installed_curseforge_file(
        &staged,
        &file,
        None,
        Some(&download),
    )
    .await;
    assert!(result.err().unwrap().to_string().contains("fingerprint"));
}

#[test]
fn curseforge_overrides_publish_after_all_entries_validate() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    let instance_path = root.path().join("instance");
    let first = instance_path.join("config/first.txt");
    let second = instance_path.join("config/second.txt");
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    std::fs::write(&first, b"old-first").unwrap();
    std::fs::write(&second, b"old-second").unwrap();
    let corrupt_payload = b"unique-corrupt-override-payload";
    write_override_test_archive(
        &archive_path,
        &[
            ("config/first.txt", b"new-first"),
            ("config/second.txt", corrupt_payload),
        ],
    );
    let mut bytes = std::fs::read(&archive_path).unwrap();
    let offset = bytes
        .windows(corrupt_payload.len())
        .position(|window| window == corrupt_payload)
        .unwrap();
    bytes[offset] ^= 0xff;
    std::fs::write(&archive_path, bytes).unwrap();

    let error =
        extract_modpack_overrides(&archive_path, &instance_path, None)
            .unwrap_err();

    assert!(
        error.to_string().contains("CRC")
            || error.to_string().contains("checksum")
    );
    assert_eq!(std::fs::read(&first).unwrap(), b"old-first");
    assert_eq!(std::fs::read(&second).unwrap(), b"old-second");
    assert!(
        !PathBuf::from(format!("{}.installing", first.display())).exists()
    );
    assert!(
        !PathBuf::from(format!("{}.installing", second.display())).exists()
    );
}

fn write_override_size_test_archive(path: &Path, entries: &[(&str, u32)]) {
    let contents = entries
        .iter()
        .map(|(name, _)| (*name, b"x".as_slice()))
        .collect::<Vec<_>>();
    write_override_test_archive(path, &contents);
    let mut bytes = std::fs::read(path).unwrap();
    let headers = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(offset, signature)| {
            (signature == b"PK\x01\x02").then_some(offset)
        })
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), entries.len() + 1);
    for (offset, (_, size)) in headers.into_iter().skip(1).zip(entries) {
        bytes[offset + 24..offset + 28]
            .copy_from_slice(&size.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn curseforge_overrides_accept_sizes_above_two_gib_up_to_shared_limit() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    for sizes in [
        [3 * 1024 * 1024 * 1024, 0, 0],
        [u32::MAX - 1, u32::MAX - 1, 4],
    ] {
        write_override_size_test_archive(
            &archive_path,
            &[("a", sizes[0]), ("b", sizes[1]), ("c", sizes[2])],
        );
        let mut archive = zip::ZipArchive::new(
            std::fs::File::open(&archive_path).unwrap(),
        )
        .unwrap();
        let manifest = read_modpack_manifest(&mut archive).unwrap();
        let tasks = collect_modpack_override_tasks(
            &mut archive,
            &manifest,
            root.path(),
            None,
        )
        .unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(
            tasks.iter().map(|task| task.size).sum::<u64>(),
            sizes.into_iter().map(u64::from).sum::<u64>(),
        );
    }
}

#[test]
fn curseforge_overrides_reject_above_shared_limit_before_writing() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    let instance_path = root.path().join("instance");
    write_override_size_test_archive(
        &archive_path,
        &[("a", u32::MAX - 1), ("b", u32::MAX - 1), ("c", 5)],
    );
    let error =
        materialize_modpack_overrides(&archive_path, &instance_path, None)
            .err()
            .unwrap()
            .to_string();
    assert!(error.contains("overrides exceed the extraction limit"));
    assert!(error.contains("8589934593 unpacked bytes"));
    assert!(error.contains("limit is 8589934592 bytes"));
    assert!(!instance_path.exists());
}

#[test]
fn curseforge_overrides_size_counts_only_final_duplicate_targets() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    write_override_size_test_archive(
        &archive_path,
        &[
            ("config/a", u32::MAX - 1),
            ("config\\a", 1),
            ("config/b", u32::MAX - 1),
            ("config/c", 5),
        ],
    );
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&archive_path).unwrap())
            .unwrap();
    let manifest = read_modpack_manifest(&mut archive).unwrap();
    let tasks = collect_modpack_override_tasks(
        &mut archive,
        &manifest,
        root.path(),
        None,
    )
    .unwrap();
    assert_eq!(tasks.len(), 3);
    let winner = tasks
        .iter()
        .find(|task| task.target == root.path().join("config/a"))
        .unwrap();
    assert_eq!(winner.index, 2);
    assert_eq!(winner.size, 1);
}

#[test]
fn curseforge_overrides_commit_successful_parallel_staging() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    let instance_path = root.path().join("instance");
    let target = instance_path.join("config/example.txt");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"old").unwrap();
    write_override_test_archive(
        &archive_path,
        &[("config/example.txt", b"replacement")],
    );

    let written =
        extract_modpack_overrides(&archive_path, &instance_path, None)
            .unwrap();

    assert_eq!(written, 1);
    assert_eq!(std::fs::read(&target).unwrap(), b"replacement");
    assert!(
        !PathBuf::from(format!("{}.installing", target.display())).exists()
    );
    assert!(
        !PathBuf::from(format!("{}.installing.previous", target.display()))
            .exists()
    );
}

#[test]
fn curseforge_overrides_restore_after_later_stage_failure() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("pack.zip");
    let instance_path = root.path().join("instance");
    let existing = instance_path.join("config/existing.txt");
    let created = instance_path.join("config/created.txt");
    std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
    std::fs::write(&existing, b"old-existing").unwrap();
    write_override_test_archive(
        &archive_path,
        &[
            ("config/existing.txt", b"new-existing"),
            ("config/created.txt", b"new-created"),
        ],
    );

    let (written, replacements) =
        materialize_modpack_overrides(&archive_path, &instance_path, None)
            .unwrap();
    assert_eq!(written, 2);
    assert_eq!(std::fs::read(&existing).unwrap(), b"new-existing");
    assert_eq!(std::fs::read(&created).unwrap(), b"new-created");

    replacements.rollback().unwrap();

    assert_eq!(std::fs::read(&existing).unwrap(), b"old-existing");
    assert!(!created.exists());
    assert!(
        !PathBuf::from(format!(
            "{}.installing.previous",
            existing.display()
        ))
        .exists()
    );
}

#[tokio::test]
async fn curseforge_database_worker_cancels_while_queue_is_empty() {
    let cancellation = CancellationToken::new();
    let context = CurseForgeVerificationContext {
        reporter: None,
        loading_bar: None,
        details: InstallPhaseDetails::Empty,
        files_done: Arc::new(AtomicU64::new(0)),
        bytes_done: Arc::new(AtomicU64::new(0)),
        active_downloads: Arc::new(AtomicU64::new(0)),
        total_files: 1,
        total_bytes: 1,
        cancellation: cancellation.clone(),
    };
    let (_sender, receiver) = mpsc::channel(1);
    let worker = spawn_curseforge_database_worker(receiver, context);

    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_millis(100), worker)
        .await
        .expect("an idle database worker should stop immediately")
        .unwrap()
        .unwrap_err();

    assert!(error.to_string().contains("canceled"));
}

#[tokio::test]
async fn curseforge_database_batch_is_retained_when_canceled_mid_batch() {
    let cancellation = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel(2);
    sender.send(7_u8).await.unwrap();
    let trigger = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        trigger.cancel();
    });

    let batch = tokio::time::timeout(
        Duration::from_millis(100),
        receive_curseforge_database_batch(&mut receiver, &cancellation),
    )
    .await
    .expect("partial database batch should be returned on cancellation")
    .unwrap()
    .unwrap();
    cancel_task.await.unwrap();

    assert_eq!(batch, vec![7]);
}

#[test]
fn active_curseforge_download_counter_is_restored_on_drop() {
    let counter = Arc::new(AtomicU64::new(0));
    {
        let _active = ActiveCurseForgeDownload::start(counter.clone());
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    let mut active = ActiveCurseForgeDownload::start(counter.clone());
    active.finish();
    active.finish();
    assert_eq!(counter.load(Ordering::Relaxed), 0);
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn failed_curseforge_verification_never_replaces_existing_file() {
    let _state = stage6_state().await;
    let directory = tempfile::tempdir().unwrap();
    let download_path = directory.path().join("new.installing.download");
    let full_path = directory.path().join("existing.jar");
    crate::util::io::write(&download_path, b"invalid replacement")
        .await
        .unwrap();
    crate::util::io::write(&full_path, b"existing content")
        .await
        .unwrap();
    let expected_sha1 =
        sha1_smol::Sha1::from(b"expected replacement").hexdigest();
    let file = stage8_curseforge_file(
        1,
        2,
        "existing.jar",
        19,
        vec![CurseForgeFileHash {
            value: expected_sha1,
            algo: 1,
        }],
        0,
    );
    let (database_tx, _database_rx) = mpsc::channel(1);

    let error = verify_and_record_curseforge_modpack_file(
        CurseForgeVerificationTask {
            instance_id: "verification-order-test".to_string(),
            relative_path: "mods/existing.jar".to_string(),
            download_path: download_path.clone(),
            full_path: full_path.clone(),
            file,
            project_type: ProjectType::Mod,
            ownership_kind:
                crate::state::instances::ContentOwnershipKind::PackManaged,
            expected_bytes: 19,
            download_result: None,
            cancellation: CancellationToken::new(),
        },
        database_tx,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().to_ascii_lowercase().contains("hash"));
    assert_eq!(
        crate::util::io::read(&full_path).await.unwrap(),
        b"existing content"
    );
    assert!(!download_path.exists());
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn canceled_curseforge_database_batch_restores_previous_file() {
    let _state = stage6_state().await;
    let directory = tempfile::tempdir().unwrap();
    let full_path = directory.path().join("content.jar");
    let previous_path =
        directory.path().join("content.jar.installing.previous");
    crate::util::io::write(&full_path, b"new content")
        .await
        .unwrap();
    crate::util::io::write(&previous_path, b"old content")
        .await
        .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let context = CurseForgeVerificationContext {
        reporter: None,
        loading_bar: None,
        details: InstallPhaseDetails::Empty,
        files_done: Arc::new(AtomicU64::new(0)),
        bytes_done: Arc::new(AtomicU64::new(0)),
        active_downloads: Arc::new(AtomicU64::new(0)),
        total_files: 1,
        total_bytes: 11,
        cancellation,
    };
    let batch = vec![CurseForgeDatabaseTask {
        instance_id: "database-rollback-test".to_string(),
        record: crate::state::instances::commands::ProjectFileRecord {
            relative_path: "mods/content.jar".to_string(),
            sha1: String::new(),
            size: 11,
            project_type: ProjectType::Mod,
            source_kind: ContentSourceKind::CurseForge,
            ownership_kind:
                crate::state::instances::ContentOwnershipKind::PackManaged,
            provider_ref: None,
            origin: true,
            known_modrinth_project_id: None,
            known_modrinth_version_id: None,
        },
        verified_pending: None,
        expected_bytes: 11,
        full_path: full_path.clone(),
        previous_path: Some(previous_path.clone()),
    }];

    persist_curseforge_database_batch(&batch, &context)
        .await
        .unwrap_err();

    assert_eq!(
        crate::util::io::read(&full_path).await.unwrap(),
        b"old content"
    );
    assert!(!previous_path.exists());
}

#[test]
fn dependency_fallback_requires_the_target_game_and_loader() {
    let mut target_file =
        stage8_curseforge_file(1, 101, "target.jar", 0, Vec::new(), 0);
    target_file.game_versions =
        vec!["1.21.1".to_string(), "NeoForge".to_string()];

    let mut newer_game_file =
        stage8_curseforge_file(1, 102, "newer-game.jar", 0, Vec::new(), 0);
    newer_game_file.game_versions =
        vec!["26.1.2".to_string(), "NeoForge".to_string()];

    let mut wrong_loader_file = stage8_curseforge_file(
        1,
        103,
        "wrong-loader.jar",
        0,
        Vec::new(),
        0,
    );
    wrong_loader_file.game_versions =
        vec!["1.21.1".to_string(), "Fabric".to_string()];

    assert!(file_matches_dependency_target(
        &target_file,
        Some("1.21.1"),
        Some(6),
    ));
    assert!(!file_matches_dependency_target(
        &newer_game_file,
        Some("1.21.1"),
        Some(6),
    ));
    assert!(!file_matches_dependency_target(
        &wrong_loader_file,
        Some("1.21.1"),
        Some(6),
    ));
}

#[test]
fn dependency_fallback_rejects_conflicting_game_version_metadata() {
    let mut conflicting_file =
        stage8_curseforge_file(1, 101, "conflicting.jar", 0, Vec::new(), 0);
    conflicting_file.game_versions =
        vec!["1.21.1".to_string(), "NeoForge".to_string()];
    conflicting_file.sortable_game_versions =
        vec![CurseForgeSortableGameVersion {
            game_version_name: "26.1.2".to_string(),
            game_version_padded: None,
            game_version: Some("26.1.2".to_string()),
            game_version_release_date: None,
            game_version_type_id: None,
        }];

    assert!(!file_matches_dependency_target(
        &conflicting_file,
        Some("1.21.1"),
        Some(6),
    ));
}

#[test]
fn dependency_selection_requires_a_minecraft_target() {
    assert_eq!(dependency_target_game_version(None), None);
    assert_eq!(dependency_target_game_version(Some("")), None);
    assert_eq!(dependency_target_game_version(Some("  ")), None);
    assert_eq!(
        dependency_target_game_version(Some("1.21.1")),
        Some("1.21.1"),
    );
}

#[test]
fn dependency_selection_prefers_a_stable_target_match_over_newer_beta() {
    let mut stable =
        stage8_curseforge_file(1, 101, "stable.jar", 0, Vec::new(), 0);
    stable.game_versions =
        vec!["1.21.1".to_string(), "NeoForge".to_string()];
    stable.release_type = 1;
    stable.file_date = "2026-08-01T00:00:00Z".to_string();

    let mut newer_beta =
        stage8_curseforge_file(1, 102, "newer-beta.jar", 0, Vec::new(), 0);
    newer_beta.game_versions =
        vec!["1.21.1".to_string(), "NeoForge".to_string()];
    newer_beta.release_type = 2;
    newer_beta.file_date = "2026-08-02T00:00:00Z".to_string();

    let selected = select_best_dependency_file(
        vec![newer_beta, stable],
        Some("1.21.1"),
        Some(6),
    )
    .unwrap();

    assert_eq!(selected.id, 101);
}

#[test]
fn waystones_fixture_selects_neoforge_balm_for_minecraft_1_21_1() {
    let mut balm_1_21_1 = stage8_curseforge_file(
        531_761,
        21_065,
        "balm-neoforge-1.21.1-21.0.65.jar",
        0,
        Vec::new(),
        0,
    );
    balm_1_21_1.game_versions =
        vec!["1.21.1".to_string(), "NeoForge".to_string()];
    balm_1_21_1.release_type = 1;

    let mut balm_26_1_2 = stage8_curseforge_file(
        531_761,
        26_102,
        "balm-neoforge-26.1.2-26.1.2.jar",
        0,
        Vec::new(),
        0,
    );
    balm_26_1_2.game_versions =
        vec!["26.1.2".to_string(), "NeoForge".to_string()];
    balm_26_1_2.release_type = 1;

    let selected = select_best_dependency_file(
        vec![balm_26_1_2, balm_1_21_1],
        Some("1.21.1"),
        Some(6),
    )
    .unwrap();
    assert_eq!(selected.file_name, "balm-neoforge-1.21.1-21.0.65.jar");
}

fn skipped_curseforge_manual_item(
    id: &str,
    project_id: &str,
    version_id: &str,
) -> crate::install::model::DownloadItemSnapshot {
    crate::install::model::DownloadItemSnapshot {
        id: id.to_string(),
        name: id.to_string(),
        project_id: Some(project_id.to_string()),
        version_id: Some(version_id.to_string()),
        status: crate::install::model::DownloadItemStatus::Skipped,
        bytes_downloaded: 0,
        bytes_total: Some(42),
        attempt: Some(1),
        max_attempts: Some(1),
        error: Some("CurseForge requires manual download".to_string()),
        manual_url: Some("https://www.curseforge.com/download".to_string()),
        request_url: None,
        source: None,
    }
}

fn waiting_manual_job_record(
    instance_id: &str,
    request: crate::install::model::InstallRequest,
) -> crate::install::store::InstallJobRecord {
    let mut state = crate::install::model::InstallJobState::new(request);
    state.record_event(InstallJobEventKind::ContentFileSkipped {
        path: "mods/one.jar".to_string(),
        reason: "manual download required".to_string(),
        project_id: Some("1".to_string()),
        version_id: Some("10".to_string()),
        manual_url: Some("https://www.curseforge.com/download".to_string()),
    });
    state.record_event(InstallJobEventKind::WaitingForUser {
        reason:
            crate::install::model::InstallPauseReason::MissingRequiredContent {
                failed_files: 1,
                paths: vec!["mods/one.jar".to_string()],
            },
    });
    state.pause_reason = Some(
        crate::install::model::InstallPauseReason::MissingRequiredContent {
            failed_files: 1,
            paths: vec!["mods/one.jar".to_string()],
        },
    );
    let now = chrono::Utc::now();
    crate::install::store::InstallJobRecord {
        id: uuid::Uuid::new_v4(),
        instance_id: Some(instance_id.to_string()),
        kind: crate::install::model::InstallJobKind::InstallContent,
        status: crate::install::model::InstallJobStatus::WaitingForUser,
        state,
        created: now,
        modified: now,
        finished: None,
        dismissed: false,
    }
}

fn apply_recovered_events(
    items: &mut [crate::install::model::DownloadItemSnapshot],
    events: &[InstallJobEventKind],
) {
    for event in events {
        let InstallJobEventKind::ContentFileRecovered { path, bytes } =
            event
        else {
            continue;
        };
        let item = items.iter_mut().find(|item| item.id == *path).unwrap();
        item.status = crate::install::model::DownloadItemStatus::Completed;
        item.bytes_downloaded = *bytes;
    }
}

fn reconcile_and_simulate_resume(
    items: &mut [crate::install::model::DownloadItemSnapshot],
    pending: &HashSet<(String, String)>,
    materialized: &HashSet<(String, String)>,
    resume_count: &mut usize,
) -> CurseForgeManualDownloadReconciliation {
    let result = curseforge_manual_download_reconciliation(
        items,
        pending,
        materialized,
    );
    apply_recovered_events(items, &result.recovered);
    let latest = curseforge_manual_download_reconciliation(
        items,
        pending,
        materialized,
    );
    if latest.should_resume() {
        *resume_count += 1;
    }
    result
}

#[cfg(not(feature = "tauri"))]
async fn stage6_state() -> std::sync::Arc<State> {
    crate::event::EventState::init().await.unwrap();
    let state_root = tempfile::tempdir().unwrap().keep();
    State::init_for_test(state_root.to_string_lossy().to_string())
        .await
        .unwrap()
}

#[cfg(not(feature = "tauri"))]
async fn create_stage6_instance(
    label: &str,
) -> (std::sync::Arc<State>, String) {
    let state = stage6_state().await;
    let created = crate::api::instance::create(
        format!("Stage 6 {label} {}", uuid::Uuid::new_v4()),
        "1.20.1".to_string(),
        ModLoader::Vanilla,
        None,
        None,
        InstanceLink::Unmanaged,
        None,
        None,
    )
    .await
    .unwrap();
    (state, created.instance.id)
}

#[cfg(not(feature = "tauri"))]
fn stage6_manual_download(
    project_id: u32,
    file_id: u32,
    file_name: &str,
    expected_bytes: &[u8],
) -> CurseForgeManualDownload {
    CurseForgeManualDownload {
        project_id,
        file_id,
        file_name: file_name.to_string(),
        ownership_kind:
            crate::state::instances::ContentOwnershipKind::PackManaged,
        operation_kind:
            crate::state::instances::ManualDownloadOperationKind::PackInstall,
        website_url: None,
        project_type: "mod".to_string(),
        project_slug: format!("stage-6-{project_id}-{file_id}"),
        target_folder: "mods".to_string(),
        hashes: vec![CurseForgeFileHash {
            value: sha1_smol::Sha1::from(expected_bytes).hexdigest(),
            algo: 1,
        }],
        file_length: expected_bytes.len() as u64,
        file_fingerprint: 0,
    }
}

fn stage8_legacy_manual_download(
    project_id: u32,
    file_id: u32,
    file_name: &str,
) -> CurseForgeManualDownload {
    CurseForgeManualDownload {
        project_id,
        file_id,
        file_name: file_name.to_string(),
        ownership_kind:
            crate::state::instances::ContentOwnershipKind::PackManaged,
        operation_kind:
            crate::state::instances::ManualDownloadOperationKind::PackInstall,
        website_url: None,
        project_type: "mod".to_string(),
        project_slug: format!("stage-8-{project_id}-{file_id}"),
        target_folder: "mods".to_string(),
        hashes: Vec::new(),
        file_length: 0,
        file_fingerprint: 0,
    }
}

fn stage8_integrity_metadata(
    download: &CurseForgeManualDownload,
    hashes: Vec<CurseForgeFileHash>,
    file_length: u64,
    file_fingerprint: u64,
) -> CurseForgeManualDownloadIntegrityMetadata {
    CurseForgeManualDownloadIntegrityMetadata {
        project_id: download.project_id,
        file_id: download.file_id,
        hashes,
        file_length,
        file_fingerprint,
    }
}

fn stage8_curseforge_file(
    project_id: u32,
    file_id: u32,
    file_name: &str,
    file_length: u64,
    hashes: Vec<CurseForgeFileHash>,
    file_fingerprint: u64,
) -> CurseForgeFile {
    CurseForgeFile {
        id: file_id,
        game_id: MINECRAFT_GAME_ID,
        mod_id: project_id,
        is_available: true,
        display_name: file_name.to_string(),
        file_name: file_name.to_string(),
        release_type: 1,
        file_status: 4,
        hashes,
        file_date: String::new(),
        file_length,
        download_count: 0,
        file_size_on_disk: Some(file_length),
        download_url: None,
        game_versions: Vec::new(),
        sortable_game_versions: Vec::new(),
        dependencies: Vec::new(),
        expose_as_alternative: None,
        parent_project_file_id: None,
        alternate_file_id: None,
        is_server_pack: None,
        server_pack_file_id: None,
        is_early_access_content: None,
        early_access_end_date: None,
        file_fingerprint,
        modules: Vec::new(),
    }
}

#[cfg(not(feature = "tauri"))]
async fn stage8_pending_keys(
    state: &State,
    instance_id: &str,
) -> HashSet<(String, String)> {
    crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
        instance_id,
        &state.pool,
    )
    .await
    .unwrap()
    .into_iter()
    .map(|download| {
        (
            download.provider_project_id,
            download.provider_release_id,
        )
    })
    .collect()
}

#[cfg(not(feature = "tauri"))]
async fn import_stage6_manual_download(
    instance_id: &str,
    download: &CurseForgeManualDownload,
    bytes: &[u8],
) {
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join(&download.file_name);
    crate::util::io::write(&source, bytes).await.unwrap();
    import_pending_manual_download_file(
        instance_id,
        download.project_id,
        download.file_id,
        source,
    )
    .await
    .unwrap();
}

#[cfg(not(feature = "tauri"))]
fn stage6_waiting_manual_job_state(
    instance_id: &str,
    items: &[(&str, &str, &str)],
) -> crate::install::model::InstallJobState {
    let mut state = crate::install::model::InstallJobState::new(
        crate::install::model::InstallRequest::InstallPackToExistingInstance {
            instance_id: instance_id.to_string(),
            location: crate::api::pack::install_from::CreatePackLocation::FromFile {
                path: PathBuf::from(format!(
                    "missing-stage-6-{}.mrpack",
                    uuid::Uuid::new_v4()
                )),
            },
            post_install_edit: None,
        },
    );
    let paths = items
        .iter()
        .map(|(path, _, _)| (*path).to_string())
        .collect::<Vec<_>>();
    for (path, project_id, file_id) in items {
        state.record_event(InstallJobEventKind::ContentFileQueued {
            path: (*path).to_string(),
            bytes_total: Some(42),
            max_attempts: 1,
        });
        state.record_event(InstallJobEventKind::ContentFileSkipped {
            path: (*path).to_string(),
            reason: "manual download required".to_string(),
            project_id: Some((*project_id).to_string()),
            version_id: Some((*file_id).to_string()),
            manual_url: Some(
                "https://www.curseforge.com/download".to_string(),
            ),
        });
    }
    let reason =
        crate::install::model::InstallPauseReason::MissingRequiredContent {
            failed_files: paths.len() as u64,
            paths,
        };
    state.pause_reason = Some(reason.clone());
    state.record_event(InstallJobEventKind::WaitingForUser { reason });
    state
}

#[cfg(not(feature = "tauri"))]
async fn insert_stage6_waiting_manual_job(
    state: &State,
    instance_id: &str,
    items: &[(&str, &str, &str)],
) -> uuid::Uuid {
    let job_id = uuid::Uuid::new_v4();
    let job_state = stage6_waiting_manual_job_state(instance_id, items);
    crate::install::store::insert(
        job_id,
        &job_state,
        crate::install::model::InstallJobStatus::WaitingForUser,
        state,
    )
    .await
    .unwrap();
    job_id
}

#[cfg(not(feature = "tauri"))]
async fn assert_stage6_job_is_unresolved(
    state: &State,
    job_id: uuid::Uuid,
    item_path: &str,
) {
    let job = crate::install::store::get_required(job_id, state)
        .await
        .unwrap();
    assert_eq!(
        job.status,
        crate::install::model::InstallJobStatus::WaitingForUser
    );
    assert_eq!(
        job.snapshot()
            .items
            .iter()
            .find(|item| item.id == item_path)
            .unwrap()
            .status,
        crate::install::model::DownloadItemStatus::Skipped
    );
    assert_eq!(
        job.state
            .events
            .iter()
            .filter(|event| matches!(
                event.kind,
                InstallJobEventKind::ContentFileRecovered { .. }
            ))
            .count(),
        0
    );
}

#[test]
fn curseforge_final_manual_completion_recovers_and_resumes() {
    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let materialized = HashSet::from([("1".to_string(), "10".to_string())]);
    let mut resume_count = 0;

    let result = reconcile_and_simulate_resume(
        &mut items,
        &HashSet::new(),
        &materialized,
        &mut resume_count,
    );

    assert_eq!(result.recovered.len(), 1);
    assert_eq!(result.materialized_exact_match_count, 1);
    assert_eq!(resume_count, 1);
    assert_eq!(
        items[0].status,
        crate::install::model::DownloadItemStatus::Completed
    );
}

#[test]
fn curseforge_missing_pending_without_materialization_does_not_resume() {
    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let mut resume_count = 0;

    let result = reconcile_and_simulate_resume(
        &mut items,
        &HashSet::new(),
        &HashSet::new(),
        &mut resume_count,
    );

    assert!(result.recovered.is_empty());
    assert_eq!(result.inconsistent.len(), 1);
    assert_eq!(
        result.inconsistent[0].reason,
        "pending_missing_but_not_materialized"
    );
    assert_eq!(resume_count, 0);
    assert_eq!(
        items[0].status,
        crate::install::model::DownloadItemStatus::Skipped
    );
}

#[test]
fn curseforge_pending_still_exists_does_not_resume() {
    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let pending = HashSet::from([("1".to_string(), "10".to_string())]);
    let mut resume_count = 0;

    let result = reconcile_and_simulate_resume(
        &mut items,
        &pending,
        &HashSet::new(),
        &mut resume_count,
    );

    assert!(result.recovered.is_empty());
    assert_eq!(result.unresolved_pending_count, 1);
    assert!(result.inconsistent.is_empty());
    assert_eq!(resume_count, 0);
}

#[test]
fn curseforge_partial_manual_completion_does_not_resume() {
    let mut items = vec![
        skipped_curseforge_manual_item("mods/a.jar", "1", "10"),
        skipped_curseforge_manual_item("mods/b.jar", "2", "20"),
        skipped_curseforge_manual_item("mods/c.jar", "3", "30"),
    ];
    let keys = [
        ("1".to_string(), "10".to_string()),
        ("2".to_string(), "20".to_string()),
        ("3".to_string(), "30".to_string()),
    ];
    let mut pending = keys.iter().cloned().collect::<HashSet<_>>();
    let mut materialized = HashSet::new();
    let mut resume_count = 0;

    for (index, key) in keys.into_iter().enumerate() {
        assert!(pending.remove(&key));
        assert!(materialized.insert(key));
        let result = reconcile_and_simulate_resume(
            &mut items,
            &pending,
            &materialized,
            &mut resume_count,
        );

        assert_eq!(result.recovered.len(), 1);
        assert!(items[..=index].iter().all(|item| {
            item.status
                == crate::install::model::DownloadItemStatus::Completed
        }));
        assert!(items[index + 1..].iter().all(|item| {
            item.status
                == crate::install::model::DownloadItemStatus::Skipped
        }));
        assert_eq!(resume_count, usize::from(index == 2));
    }
}

#[test]
fn curseforge_repeated_reconciliation_is_idempotent() {
    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let materialized = HashSet::from([("1".to_string(), "10".to_string())]);
    let mut resume_count = 0;
    let mut waiting_for_user = true;
    let mut recovered_count = 0;

    for _ in 0..3 {
        if !waiting_for_user {
            continue;
        }
        let result = reconcile_and_simulate_resume(
            &mut items,
            &HashSet::new(),
            &materialized,
            &mut resume_count,
        );
        recovered_count += result.recovered.len();
        if resume_count > 0 {
            waiting_for_user = false;
        }
    }

    assert_eq!(recovered_count, 1);
    assert_eq!(resume_count, 1);
}

#[test]
fn curseforge_startup_selects_only_reconcilable_jobs() {
    let curseforge_job = waiting_manual_job_record(
        "curseforge-instance",
        crate::install::model::InstallRequest::InstallCurseForgeContent {
            request: CurseForgeInstallRequest {
                instance_id: "curseforge-instance".to_string(),
                project_id: 1,
                file_id: 10,
                project_type: "mod".to_string(),
                ownership_kind:
                    crate::state::instances::ContentOwnershipKind::PackManaged,
                manual_operation_kind: crate::state::instances::ManualDownloadOperationKind::PackInstall,
                game_version: None,
                mod_loader_type: None,
                world_name: None,
                install_dependencies: true,
                excluded_dependency_project_ids: Vec::new(),
					force_dependency_project_ids: Vec::new(),
                dependency_plan_id: None,
                defer_persistence: false,
                verification_tx: None,
                pre_resolved_relative_path: None,
    expected_file_name: None,
            },
            display_title: "CurseForge".to_string(),
            display_icon: None,
        },
    );
    let modrinth_job = waiting_manual_job_record(
        "modrinth-instance",
        crate::install::model::InstallRequest::InstallPackToExistingInstance {
            instance_id: "modrinth-instance".to_string(),
            location: crate::api::pack::install_from::CreatePackLocation::FromVersionId {
                project_id: "project".to_string(),
                version_id: "version".to_string(),
                title: "Modrinth".to_string(),
                icon_url: None,
            },
            post_install_edit: None,
        },
    );
    assert_eq!(
        curseforge_waiting_job_instance_ids(&[
            curseforge_job,
            modrinth_job,
        ]),
        vec!["curseforge-instance".to_string()]
    );

    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let materialized = HashSet::from([("1".to_string(), "10".to_string())]);
    let mut resume_count = 0;

    let result = reconcile_and_simulate_resume(
        &mut items,
        &HashSet::new(),
        &materialized,
        &mut resume_count,
    );

    assert_eq!(result.recovered.len(), 1);
    assert_eq!(resume_count, 1);
}

#[test]
fn curseforge_wrong_instance_or_release_never_recovers() {
    let mut items =
        vec![skipped_curseforge_manual_item("mods/one.jar", "1", "10")];
    let other_instance_or_release = HashSet::from([
        ("1".to_string(), "11".to_string()),
        ("2".to_string(), "10".to_string()),
    ]);
    let mut resume_count = 0;

    let result = reconcile_and_simulate_resume(
        &mut items,
        &HashSet::new(),
        &other_instance_or_release,
        &mut resume_count,
    );

    assert!(result.recovered.is_empty());
    assert_eq!(result.inconsistent.len(), 1);
    assert_eq!(resume_count, 0);
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_pending_is_scoped_by_instance() {
    let (state, instance_a) = create_stage6_instance("scope A").await;
    let (_, instance_b) = create_stage6_instance("scope B").await;
    let completed = stage6_manual_download(
        101,
        1001,
        "scope-one.jar",
        b"scope-one-content",
    );
    let other_release = stage6_manual_download(
        101,
        1002,
        "scope-two.jar",
        b"scope-two-content",
    );
    persist_manual_download(&instance_a, &completed)
        .await
        .unwrap();
    persist_manual_download(&instance_a, &other_release)
        .await
        .unwrap();
    persist_manual_download(&instance_b, &completed)
        .await
        .unwrap();
    let job_a = insert_stage6_waiting_manual_job(
        &state,
        &instance_a,
        &[
            ("mods/scope-one.jar", "101", "1001"),
            ("mods/scope-two.jar", "101", "1002"),
        ],
    )
    .await;
    let job_b = insert_stage6_waiting_manual_job(
        &state,
        &instance_b,
        &[("mods/scope-one.jar", "101", "1001")],
    )
    .await;

    import_stage6_manual_download(
        &instance_a,
        &completed,
        b"scope-one-content",
    )
    .await;

    let pending_a = crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
        &instance_a,
        &state.pool,
    )
    .await
    .unwrap();
    let pending_b = crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
        &instance_b,
        &state.pool,
    )
    .await
    .unwrap();
    assert_eq!(pending_a.len(), 1);
    assert_eq!(pending_a[0].provider_project_id, "101");
    assert_eq!(pending_a[0].provider_release_id, "1002");
    assert_eq!(pending_b.len(), 1);
    assert_eq!(pending_b[0].provider_project_id, "101");
    assert_eq!(pending_b[0].provider_release_id, "1001");

    let reconciled_a = crate::install::store::get_required(job_a, &state)
        .await
        .unwrap();
    assert_eq!(
        reconciled_a.status,
        crate::install::model::InstallJobStatus::WaitingForUser
    );
    let items_a = reconciled_a.snapshot().items;
    assert_eq!(
        items_a
            .iter()
            .find(|item| item.id == "mods/scope-one.jar")
            .unwrap()
            .status,
        crate::install::model::DownloadItemStatus::Completed
    );
    assert_eq!(
        items_a
            .iter()
            .find(|item| item.id == "mods/scope-two.jar")
            .unwrap()
            .status,
        crate::install::model::DownloadItemStatus::Skipped
    );
    assert_eq!(
        reconciled_a
            .state
            .events
            .iter()
            .filter(|event| matches!(
                event.kind,
                InstallJobEventKind::ContentFileRecovered { .. }
            ))
            .count(),
        1
    );
    assert_stage6_job_is_unresolved(&state, job_b, "mods/scope-one.jar")
        .await;
    crate::install::store::dismiss(job_a, &state).await.unwrap();
    crate::install::store::dismiss(job_b, &state).await.unwrap();
    crate::api::instance::remove(&instance_a).await.unwrap();
    crate::api::instance::remove(&instance_b).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_missing_pending_without_materialization_keeps_real_job_waiting()
 {
    let (state, instance_id) =
        create_stage6_instance("missing materialization").await;
    let job_id = insert_stage6_waiting_manual_job(
        &state,
        &instance_id,
        &[("mods/missing.jar", "201", "2001")],
    )
    .await;

    reconcile_curseforge_waiting_jobs_for_instance_with_state(
        &instance_id,
        &state,
    )
    .await
    .unwrap();

    assert_stage6_job_is_unresolved(&state, job_id, "mods/missing.jar")
        .await;
    crate::install::store::dismiss(job_id, &state)
        .await
        .unwrap();
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_pending_still_exists_keeps_real_job_waiting() {
    let (state, instance_id) = create_stage6_instance("pending").await;
    let download = stage6_manual_download(
        301,
        3001,
        "pending.jar",
        b"pending-content",
    );
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let job_id = insert_stage6_waiting_manual_job(
        &state,
        &instance_id,
        &[("mods/pending.jar", "301", "3001")],
    )
    .await;

    reconcile_curseforge_waiting_jobs_for_instance_with_state(
        &instance_id,
        &state,
    )
    .await
    .unwrap();

    assert_stage6_job_is_unresolved(&state, job_id, "mods/pending.jar")
        .await;
    crate::install::store::dismiss(job_id, &state)
        .await
        .unwrap();
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_wrong_release_keeps_real_job_waiting() {
    let (state, instance_id) =
        create_stage6_instance("wrong release").await;
    let other_release = stage6_manual_download(
        401,
        4002,
        "other-release.jar",
        b"other-release-content",
    );
    persist_manual_download(&instance_id, &other_release)
        .await
        .unwrap();
    import_stage6_manual_download(
        &instance_id,
        &other_release,
        b"other-release-content",
    )
    .await;
    let job_id = insert_stage6_waiting_manual_job(
        &state,
        &instance_id,
        &[("mods/target-release.jar", "401", "4001")],
    )
    .await;

    reconcile_curseforge_waiting_jobs_for_instance_with_state(
        &instance_id,
        &state,
    )
    .await
    .unwrap();

    assert_stage6_job_is_unresolved(
        &state,
        job_id,
        "mods/target-release.jar",
    )
    .await;
    crate::install::store::dismiss(job_id, &state)
        .await
        .unwrap();
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_startup_reconciliation_recovers_and_resumes_once() {
    let (state, instance_id) = create_stage6_instance("startup").await;
    let download = stage6_manual_download(
        501,
        5001,
        "startup.jar",
        b"startup-content",
    );
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    import_stage6_manual_download(
        &instance_id,
        &download,
        b"startup-content",
    )
    .await;
    let job_id = insert_stage6_waiting_manual_job(
        &state,
        &instance_id,
        &[("mods/startup.jar", "501", "5001")],
    )
    .await;
    let resume_count =
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let state_for_resume = std::sync::Arc::clone(&state);
    let resume_count_for_callback = std::sync::Arc::clone(&resume_count);
    let mut resume_job = move |job_id| {
        let state = std::sync::Arc::clone(&state_for_resume);
        let resume_count =
            std::sync::Arc::clone(&resume_count_for_callback);
        async move {
            let current =
                crate::install::store::get_required(job_id, &state).await?;
            let claimed = crate::install::store::update_status_if(
                job_id,
                crate::install::model::InstallJobStatus::WaitingForUser,
                crate::install::model::InstallJobStatus::Queued,
                &current.state,
                &state,
            )
            .await?;
            assert!(
                claimed.is_some(),
                "resume status claim must succeed once"
            );
            resume_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    };

    for _ in 0..3 {
        reconcile_persisted_curseforge_waiting_jobs_with_resume(
            &state,
            &mut resume_job,
        )
        .await
        .unwrap();
    }

    let reconciled = crate::install::store::get_required(job_id, &state)
        .await
        .unwrap();
    assert_eq!(
        reconciled.status,
        crate::install::model::InstallJobStatus::Queued
    );
    assert_eq!(
        reconciled
            .snapshot()
            .items
            .iter()
            .find(|item| item.id == "mods/startup.jar")
            .unwrap()
            .status,
        crate::install::model::DownloadItemStatus::Completed
    );
    assert_eq!(
        reconciled
            .state
            .events
            .iter()
            .filter(|event| matches!(
                event.kind,
                InstallJobEventKind::ContentFileRecovered { .. }
            ))
            .count(),
        1
    );
    assert_eq!(resume_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    crate::install::store::dismiss(job_id, &state)
        .await
        .unwrap();
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[test]
fn unified_search_hit_accepts_null_gallery() {
    let hit: UnifiedSearchHit = serde_json::from_value(serde_json::json!({
        "provider": "curseforge",
        "project_id": "250419",
        "slug": null,
        "author": "DarkhaxDev",
        "author_url": null,
        "title": "Enchantment Descriptions",
        "description": "description",
        "project_type": "mod",
        "categories": [],
        "versions": [],
        "downloads": 0,
        "icon_url": null,
        "date_created": "2026-01-01T00:00:00Z",
        "date_modified": "2026-01-01T00:00:00Z",
        "latest_version": null,
        "gallery": null,
        "website_url": null,
        "source_url": null,
        "allow_mod_distribution": null,
    }))
    .unwrap();

    assert!(hit.gallery.is_empty());
}

#[test]
fn curseforge_file_accepts_null_modules() {
    let file: CurseForgeFile = serde_json::from_value(serde_json::json!({
        "id": 4031925,
        "gameId": 432,
        "modId": 250419,
        "isAvailable": true,
        "displayName": "old.jar",
        "fileName": "old.jar",
        "releaseType": 1,
        "fileStatus": 4,
        "fileDate": "2026-01-01T00:00:00Z",
        "fileLength": 1,
        "downloadCount": 0,
        "fileFingerprint": 0,
        "modules": null,
    }))
    .unwrap();

    assert!(file.modules.is_empty());
}

#[test]
fn fingerprint_ignores_curseforge_whitespace() {
    assert_eq!(
        compute_fingerprint(b"abc\r\n def\t"),
        compute_fingerprint(b"abcdef")
    );
}

#[test]
fn streaming_fingerprint_matches_reference_across_chunk_boundaries() {
    let data = (0..4099)
        .map(|index| match index % 31 {
            0 => b' ',
            1 => b'\n',
            2 => b'\r',
            3 => b'\t',
            _ => (index % 251) as u8,
        })
        .collect::<Vec<_>>();
    let normalized = data
        .iter()
        .copied()
        .filter(|byte| !is_curseforge_fingerprint_whitespace(*byte))
        .collect::<Vec<_>>();
    let expected = murmur2(&normalized, 1);

    for chunk_size in [1, 2, 3, 4, 5, 31, 256, 1024] {
        let mut hasher =
            CurseForgeFingerprintHasher::new(normalized.len() as u32);
        for chunk in data.chunks(chunk_size) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finish(), expected, "chunk size {chunk_size}");
    }
    assert_eq!(compute_fingerprint(&data), expected);
}

#[tokio::test]
async fn file_fingerprint_streams_sha1_and_supports_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-fingerprint.jar");
    let mut data = vec![b'a'; 256 * 1024 + 17];
    data[3] = b' ';
    data[256 * 1024 - 1] = b'\n';
    crate::util::io::write(&path, &data).await.unwrap();

    let (size, sha1, fingerprint) =
        fingerprint_and_sha1_file(&path, None).await.unwrap();
    assert_eq!(size, data.len() as u64);
    assert_eq!(sha1, sha1_smol::Sha1::from(&data).hexdigest());
    assert_eq!(fingerprint, compute_fingerprint(&data));

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = fingerprint_and_sha1_file(&path, Some(&cancellation))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("canceled"));
}

#[test]
fn project_types_are_provider_qualified() {
    assert_eq!(project_type_for_class(Some(6)), "mod");
    assert_eq!(project_type_for_class(Some(4471)), "modpack");
    assert_eq!(project_type_for_class(Some(6552)), "shader");
    assert_eq!(project_type_for_class(Some(6945)), "datapack");
}

#[test]
fn official_only_requests_exclude_mirror_routes() {
    let routes =
        request_routes("/v1/mods/search", MirrorPolicy::OfficialOnly);

    assert!(
        routes
            .iter()
            .all(|route| route.source == RequestRouteSource::Official)
    );
}

#[test]
fn mirror_not_found_tries_next_route() {
    let route = RequestRoute {
        url: String::new(),
        use_api_key: false,
        use_system_proxy: false,
        source: RequestRouteSource::Mirror,
    };

    assert!(should_try_next_route(&route, StatusCode::NOT_FOUND, true));
}

#[test]
fn official_forbidden_response_tries_proxy_route() {
    let route = RequestRoute {
        url: String::new(),
        use_api_key: true,
        use_system_proxy: false,
        source: RequestRouteSource::Official,
    };

    assert!(should_try_next_route(&route, StatusCode::FORBIDDEN, true));
}

#[test]
fn projects_accept_negative_popularity_ranks() {
    let project = serde_json::from_value::<CurseForgeProject>(json!({
        "id": 1,
        "gameId": MINECRAFT_GAME_ID,
        "name": "Fixture",
        "slug": "fixture",
        "links": {},
        "summary": "Fixture project",
        "status": 4,
        "downloadCount": 0,
        "isFeatured": false,
        "primaryCategoryId": 6,
        "categories": [],
        "classId": 6,
        "authors": [],
        "logo": null,
        "screenshots": [],
        "mainFileId": 0,
        "latestFiles": [],
        "latestFilesIndexes": [],
        "dateCreated": "2026-01-01T00:00:00Z",
        "dateModified": "2026-01-01T00:00:00Z",
        "dateReleased": "2026-01-01T00:00:00Z",
        "allowModDistribution": true,
        "gamePopularityRank": -10,
        "isAvailable": true
    }))
    .unwrap();

    assert_eq!(project.game_popularity_rank, Some(-10));
}

#[test]
fn category_cache_can_be_filtered_for_each_project_class() {
    let categories = vec![
        category(6, None, true),
        category(406, Some(6), false),
        category(4471, None, true),
        category(4481, Some(4471), false),
    ];

    let mods = filter_categories(categories.clone(), Some(6));
    assert_eq!(
        mods.iter().map(|category| category.id).collect::<Vec<_>>(),
        vec![6, 406]
    );

    let modpacks = filter_categories(categories, Some(4471));
    assert_eq!(
        modpacks
            .iter()
            .map(|category| category.id)
            .collect::<Vec<_>>(),
        vec![4471, 4481]
    );
}

fn category(
    id: u32,
    class_id: Option<u32>,
    is_class: bool,
) -> CurseForgeCategory {
    CurseForgeCategory {
        id,
        game_id: MINECRAFT_GAME_ID,
        name: id.to_string(),
        slug: id.to_string(),
        url: String::new(),
        icon_url: None,
        date_modified: String::new(),
        is_class: Some(is_class),
        class_id,
        parent_category_id: class_id,
        display_index: Some(0),
    }
}

#[test]
fn manual_downloads_open_the_official_download_page() {
    assert_eq!(
        curseforge_file_page_url(
            Some("https://www.curseforge.com/minecraft/mc-mods/example"),
            12345,
        ),
        Some(
            "https://www.curseforge.com/minecraft/mc-mods/example/download/12345"
                .to_string()
        )
    );
    assert_eq!(
        curseforge_file_page_url(
            Some("https://www.curseforge.com/minecraft/mc-mods/example/files/12345?tab=files"),
            12345,
        ),
        Some(
            "https://www.curseforge.com/minecraft/mc-mods/example/download/12345"
                .to_string()
        )
    );
    assert_eq!(
        curseforge_file_page_url(
            Some("https://example.com/project"),
            12345
        ),
        Some("https://example.com/project".to_string())
    );
}

#[test]
fn browser_duplicate_download_names_match_the_expected_file() {
    assert!(crate::util::downloads::browser_download_file_name_matches(
        "example-mod (1).jar",
        "example-mod.jar"
    ));
    assert!(crate::util::downloads::browser_download_file_name_matches(
        "EXAMPLE-MOD.JAR",
        "example-mod.jar"
    ));
    assert!(!crate::util::downloads::browser_download_file_name_matches(
        "example-mod-fabric.jar",
        "example-mod.jar"
    ));
    assert!(!crate::util::downloads::browser_download_file_name_matches(
        "example-mod.jar.crdownload",
        "example-mod.jar"
    ));
}

#[tokio::test]
async fn curseforge_metadata_less_exact_name_is_not_trusted() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("example-mod.jar");
    crate::util::io::write(&path, b"wrong exact-name jar")
        .await
        .unwrap();
    let download = stage8_legacy_manual_download(1, 2, "example-mod.jar");

    let error = verify_manual_download_candidate_with_integrity(
        &path, &download, true,
    )
    .await
    .expect_err("filename alone must not verify a candidate");

    assert!(matches!(
        error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "The required CurseForge file has no usable integrity metadata"
    ));
}

#[tokio::test]
async fn curseforge_hydration_is_scoped_to_exact_release() {
    let download =
        stage8_legacy_manual_download(11, 101, "exact-release.jar");
    let error = ensure_manual_download_integrity_metadata_with(
        &download,
        |project_id, file_id| async move {
            assert_eq!(project_id, 11);
            assert_eq!(file_id, 101);
            Ok(CurseForgeManualDownloadIntegrityMetadata {
                project_id,
                file_id: 102,
                hashes: vec![CurseForgeFileHash {
                    value: "wrong-release-sha1".to_string(),
                    algo: 1,
                }],
                file_length: 1,
                file_fingerprint: 0,
            })
        },
    )
    .await
    .expect_err("metadata for another file must be rejected");

    assert!(matches!(
        error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "CurseForge returned metadata for a different project or file"
    ));
}

#[tokio::test]
async fn curseforge_existing_integrity_skips_hydration() {
    let mut sha1_download =
        stage8_legacy_manual_download(12, 201, "existing-sha1.jar");
    sha1_download.hashes = vec![CurseForgeFileHash {
        value: "existing-sha1".to_string(),
        algo: 1,
    }];
    let hydrated = ensure_manual_download_integrity_metadata_with(
        &sha1_download,
        |_, _| async {
            Err(ErrorKind::InputError(
                "metadata resolver must not run".to_string(),
            )
            .into())
        },
    )
    .await
    .unwrap();
    assert_eq!(hydrated.hashes[0].value, "existing-sha1");

    let mut fingerprint_download =
        stage8_legacy_manual_download(12, 202, "existing-fingerprint.jar");
    fingerprint_download.file_fingerprint = 12345;
    let hydrated = ensure_manual_download_integrity_metadata_with(
        &fingerprint_download,
        |_, _| async {
            Err(ErrorKind::InputError(
                "metadata resolver must not run".to_string(),
            )
            .into())
        },
    )
    .await
    .unwrap();
    assert_eq!(hydrated.file_fingerprint, 12345);
}

#[tokio::test]
async fn curseforge_hydrated_fingerprint_verifies_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fingerprint.jar");
    let bytes = b"legacy fingerprint content";
    crate::util::io::write(&path, bytes).await.unwrap();
    let download =
        stage8_legacy_manual_download(13, 301, "fingerprint.jar");
    let metadata = stage8_integrity_metadata(
        &download,
        Vec::new(),
        bytes.len() as u64,
        compute_fingerprint(bytes) as u64,
    );
    let hydrated = ensure_manual_download_integrity_metadata_with(
        &download,
        move |project_id, file_id| async move {
            assert_eq!((project_id, file_id), (13, 301));
            Ok(metadata)
        },
    )
    .await
    .unwrap();

    assert!(
        verify_manual_download_candidate_with_integrity(
            &path, &hydrated, true,
        )
        .await
        .unwrap()
        .is_some()
    );
}

#[tokio::test]
async fn curseforge_manual_materialization_is_copy_isolated() {
    let source_directory = tempfile::tempdir().unwrap();
    let destination_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join("manual.jar");
    let destination = destination_directory.path().join("manual.jar");
    let verified_bytes = b"verified manual bytes";
    crate::util::io::write(&source, verified_bytes)
        .await
        .unwrap();
    let (verified_size, verified_sha1) =
        sha1_file_async(&source).await.unwrap();

    crate::state::materialize_verified_project_download_copy(
        &source,
        &destination,
        verified_size,
        &verified_sha1,
    )
    .await
    .unwrap();
    crate::util::io::write(&source, b"mutated source content")
        .await
        .unwrap();

    assert_eq!(
        crate::util::io::read(&destination).await.unwrap(),
        verified_bytes
    );
    assert_eq!(
        sha1_file_async(&destination).await.unwrap(),
        (verified_size, verified_sha1)
    );
}

#[tokio::test]
async fn curseforge_fingerprint_recognition_rechecks_current_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recognized.jar");
    let matched_bytes = b"matched fingerprint bytes";
    let expected_fingerprint = compute_fingerprint(matched_bytes) as u64;
    crate::util::io::write(&path, matched_bytes).await.unwrap();
    assert!(
        verify_recognized_curseforge_file(&path, expected_fingerprint)
            .await
            .unwrap()
            .is_some()
    );

    crate::util::io::write(&path, b"changed after fingerprint match")
        .await
        .unwrap();
    assert!(
        verify_recognized_curseforge_file(&path, expected_fingerprint)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn curseforge_localized_candidate_still_requires_integrity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("[测试]example-mod (1).jar");
    crate::util::io::write(&path, b"expected curseforge file")
        .await
        .unwrap();
    let (_, sha1) = sha1_file_async(&path).await.unwrap();
    let download = CurseForgeManualDownload {
        project_id: 1,
        file_id: 2,
        file_name: "example-mod.jar".to_string(),
        ownership_kind:
            crate::state::instances::ContentOwnershipKind::PackManaged,
        operation_kind:
            crate::state::instances::ManualDownloadOperationKind::PackInstall,
        website_url: None,
        project_type: "mod".to_string(),
        project_slug: String::new(),
        target_folder: "mods".to_string(),
        hashes: vec![CurseForgeFileHash {
            value: sha1,
            algo: 1,
        }],
        file_length: 24,
        file_fingerprint: 0,
    };

    assert!(
        find_manual_download_candidate(directory.path(), &download)
            .await
            .unwrap()
            .is_some()
    );
    crate::util::io::write(&path, b"same-length-wrong-bytes!")
        .await
        .unwrap();
    assert!(
        find_manual_download_candidate(directory.path(), &download)
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_legacy_pending_imports_after_exact_metadata_hydration()
{
    let (state, instance_id) =
        create_stage6_instance("legacy hydration").await;
    let download =
        stage8_legacy_manual_download(14, 401, "legacy-hydration.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join(&download.file_name);
    let bytes = b"legacy hydrated content";
    crate::util::io::write(&source, bytes).await.unwrap();
    let metadata = stage8_integrity_metadata(
        &download,
        vec![CurseForgeFileHash {
            value: sha1_smol::Sha1::from(bytes).hexdigest(),
            algo: 1,
        }],
        bytes.len() as u64,
        0,
    );

    let imported =
        import_pending_manual_download_file_with_integrity_resolver(
            &instance_id,
            download.project_id,
            download.file_id,
            source.clone(),
            move |project_id, file_id| async move {
                assert_eq!((project_id, file_id), (14, 401));
                Ok(metadata)
            },
        )
        .await
        .unwrap();

    assert!(source.exists());
    assert_eq!(imported.relative_path, "mods/legacy-hydration.jar");
    assert!(
        crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
            &instance_id,
            &state.pool,
        )
        .await
        .unwrap()
        .is_empty()
    );
    let snapshot = crate::api::instance::get_content_snapshot(&instance_id)
        .await
        .unwrap();
    assert!(snapshot.items.iter().any(|item| {
        item.provider_project_id.as_deref() == Some("14")
            && item.provider_release_id.as_deref() == Some("401")
            && item.materialization_state
                == crate::state::instances::PackMemberMaterializationState::Present
            && item.content.is_some()
    }));
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_missing_integrity_after_hydration_keeps_pending_unresolved()
 {
    let (state, instance_id) =
        create_stage6_instance("missing hydrated integrity").await;
    let download =
        stage8_legacy_manual_download(15, 501, "missing-integrity.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let job_id = insert_stage6_waiting_manual_job(
        &state,
        &instance_id,
        &[("mods/missing-integrity.jar", "15", "501")],
    )
    .await;
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join(&download.file_name);
    crate::util::io::write(&source, b"unverifiable content")
        .await
        .unwrap();
    let metadata = stage8_integrity_metadata(&download, Vec::new(), 20, 0);

    let fetch_error =
        import_pending_manual_download_file_with_integrity_resolver(
            &instance_id,
            download.project_id,
            download.file_id,
            source.clone(),
            |project_id, file_id| async move {
                assert_eq!((project_id, file_id), (15, 501));
                Err(ErrorKind::InputError(
                    "simulated CurseForge metadata fetch failure"
                        .to_string(),
                )
                .into())
            },
        )
        .await
        .expect_err("metadata fetch failures must block import");
    assert!(matches!(
        fetch_error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "simulated CurseForge metadata fetch failure"
    ));

    let error =
        import_pending_manual_download_file_with_integrity_resolver(
            &instance_id,
            download.project_id,
            download.file_id,
            source.clone(),
            move |project_id, file_id| async move {
                assert_eq!((project_id, file_id), (15, 501));
                Ok(metadata)
            },
        )
        .await
        .expect_err("missing authoritative integrity must block import");

    assert!(matches!(
        error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "The required CurseForge file has no usable integrity metadata"
    ));
    assert!(source.exists());
    assert_eq!(
        crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
            &instance_id,
            &state.pool,
        )
        .await
        .unwrap()
        .len(),
        1
    );
    let target = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join("mods/missing-integrity.jar");
    assert!(!target.exists());
    assert_stage6_job_is_unresolved(
        &state,
        job_id,
        "mods/missing-integrity.jar",
    )
    .await;
    let job = crate::install::store::get_required(job_id, &state)
        .await
        .unwrap();
    assert_eq!(
        job.state
            .events
            .iter()
            .filter(|event| matches!(
                event.kind,
                InstallJobEventKind::ContentFileRecovered { .. }
            ))
            .count(),
        0
    );
    crate::install::store::dismiss(job_id, &state)
        .await
        .unwrap();
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_generic_record_does_not_complete_pending() {
    let (state, instance_id) =
        create_stage6_instance("generic record provenance").await;
    let download =
        stage8_legacy_manual_download(16, 601, "generic-record.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let relative_path = "mods/generic-record.jar";
    let full_path = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"generic recorded bytes";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let (_, sha1) = sha1_file_async(&full_path).await.unwrap();
    let provider_ref = ContentProviderRef::CurseForge {
        project_id: CurseForgeProjectId::new(16).unwrap(),
        file_id: Some(CurseForgeFileId::new(601).unwrap()),
    };

    crate::state::record_project_file_atomic(
        &instance_id,
        relative_path,
        &sha1,
        bytes.len() as u64,
        ProjectType::Mod,
        ContentSourceKind::CurseForge,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        Some(&provider_ref),
        true,
        None,
        &state,
    )
    .await
    .unwrap();

    assert!(
        stage8_pending_keys(&state, &instance_id)
            .await
            .contains(&("16".to_string(), "601".to_string()))
    );
    let snapshot = crate::api::instance::get_content_snapshot(&instance_id)
        .await
        .unwrap();
    assert!(snapshot.items.iter().any(|item| {
        item.provider_project_id.as_deref() == Some("16")
            && item.provider_release_id.as_deref() == Some("601")
            && item.materialization_state
                == crate::state::instances::PackMemberMaterializationState::Present
            && item.content.is_some()
    }));
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_verified_record_completes_only_exact_pending() {
    let (state, instance_a) =
        create_stage6_instance("verified record A").await;
    let (_, instance_b) = create_stage6_instance("verified record B").await;
    let file_1 = stage8_legacy_manual_download(17, 701, "one.jar");
    let file_2 = stage8_legacy_manual_download(17, 702, "two.jar");
    persist_manual_download(&instance_a, &file_1).await.unwrap();
    persist_manual_download(&instance_a, &file_2).await.unwrap();
    persist_manual_download(&instance_b, &file_1).await.unwrap();
    let relative_path = "mods/one.jar";
    let full_path = crate::api::instance::get_full_path(&instance_a)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"verified exact record";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let (_, sha1) = sha1_file_async(&full_path).await.unwrap();

    crate::state::record_verified_curseforge_project_file_atomic(
        &instance_a,
        relative_path,
        &sha1,
        bytes.len() as u64,
        ProjectType::Mod,
        ContentSourceKind::CurseForge,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        CurseForgeProjectId::new(17).unwrap(),
        CurseForgeFileId::new(701).unwrap(),
        true,
        &state,
    )
    .await
    .unwrap();

    assert_eq!(
        stage8_pending_keys(&state, &instance_a).await,
        HashSet::from([("17".to_string(), "702".to_string())])
    );
    assert_eq!(
        stage8_pending_keys(&state, &instance_b).await,
        HashSet::from([("17".to_string(), "701".to_string())])
    );
    crate::api::instance::remove(&instance_a).await.unwrap();
    crate::api::instance::remove(&instance_b).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_batch_completes_only_authoritatively_verified_pending()
{
    let (state, instance_id) =
        create_stage6_instance("verified record batch").await;
    let verified = stage8_legacy_manual_download(170, 701, "verified.jar");
    let generic = stage8_legacy_manual_download(170, 702, "generic.jar");
    persist_manual_download(&instance_id, &verified)
        .await
        .unwrap();
    persist_manual_download(&instance_id, &generic)
        .await
        .unwrap();
    let records = [
        crate::state::instances::commands::ProjectFileRecord {
            relative_path: "mods/verified.jar".to_string(),
            sha1: "verified-sha1".to_string(),
            size: 10,
            project_type: ProjectType::Mod,
            source_kind: ContentSourceKind::CurseForge,
            ownership_kind:
                crate::state::instances::ContentOwnershipKind::PackManaged,
            provider_ref: Some(ContentProviderRef::CurseForge {
                project_id: CurseForgeProjectId::new(170).unwrap(),
                file_id: Some(CurseForgeFileId::new(701).unwrap()),
            }),
            origin: true,
            known_modrinth_project_id: None,
            known_modrinth_version_id: None,
        },
        crate::state::instances::commands::ProjectFileRecord {
            relative_path: "mods/generic.jar".to_string(),
            sha1: "generic-sha1".to_string(),
            size: 11,
            project_type: ProjectType::Mod,
            source_kind: ContentSourceKind::CurseForge,
            ownership_kind:
                crate::state::instances::ContentOwnershipKind::PackManaged,
            provider_ref: Some(ContentProviderRef::CurseForge {
                project_id: CurseForgeProjectId::new(170).unwrap(),
                file_id: Some(CurseForgeFileId::new(702).unwrap()),
            }),
            origin: true,
            known_modrinth_project_id: None,
            known_modrinth_version_id: None,
        },
    ];

    crate::state::instances::commands::record_project_files_with_verified_curseforge_atomic(
        &instance_id,
        &records,
        &[(
            CurseForgeProjectId::new(170).unwrap(),
            CurseForgeFileId::new(701).unwrap(),
        )],
        &state,
    )
    .await
    .unwrap();

    assert_eq!(
        stage8_pending_keys(&state, &instance_id).await,
        HashSet::from([("170".to_string(), "702".to_string())])
    );
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_manual_materialization_detects_source_change() {
    let (state, instance_id) =
        create_stage6_instance("manual copy TOCTOU").await;
    let download = stage8_legacy_manual_download(18, 801, "toctou.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join("toctou.jar");
    crate::util::io::write(&source, b"original verified bytes")
        .await
        .unwrap();
    let (verified_size, verified_sha1) =
        sha1_file_async(&source).await.unwrap();
    crate::util::io::write(&source, b"changed after verification")
        .await
        .unwrap();
    let destination = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join("mods/toctou.jar");
    let previous_bytes = b"previous destination";
    crate::util::io::write(&destination, previous_bytes)
        .await
        .unwrap();

    crate::state::materialize_verified_project_download_copy(
        &source,
        &destination,
        verified_size,
        &verified_sha1,
    )
    .await
    .expect_err("changed source must fail copied-byte identity");

    assert_eq!(
        crate::util::io::read(&destination).await.unwrap(),
        previous_bytes
    );
    assert!(
        stage8_pending_keys(&state, &instance_id)
            .await
            .contains(&("18".to_string(), "801".to_string()))
    );
    let snapshot = crate::api::instance::get_content_snapshot(&instance_id)
        .await
        .unwrap();
    assert!(!snapshot.items.iter().any(|item| {
        item.provider_project_id.as_deref() == Some("18")
            && item.provider_release_id.as_deref() == Some("801")
            && item.materialization_state
                == crate::state::instances::PackMemberMaterializationState::Present
            && item.content.is_some()
    }));
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_automatic_sha1_record_can_complete_pending() {
    let (state, instance_id) =
        create_stage6_instance("automatic SHA1").await;
    let download =
        stage8_legacy_manual_download(19, 901, "automatic-sha1.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let relative_path = "mods/automatic-sha1.jar";
    let full_path = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"automatic SHA1 bytes";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let (_, sha1) = sha1_file_async(&full_path).await.unwrap();
    let file = stage8_curseforge_file(
        19,
        901,
        "automatic-sha1.jar",
        bytes.len() as u64,
        vec![CurseForgeFileHash {
            value: sha1,
            algo: 1,
        }],
        0,
    );

    record_installed_curseforge_file_with_permit(
        &instance_id,
        relative_path,
        &full_path,
        &file,
        ProjectType::Mod,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        None,
        &state,
    )
    .await
    .unwrap();

    assert!(stage8_pending_keys(&state, &instance_id).await.is_empty());
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_automatic_fingerprint_record_can_complete_pending() {
    let (state, instance_id) =
        create_stage6_instance("automatic fingerprint").await;
    let download = stage8_legacy_manual_download(
        20,
        1001,
        "automatic-fingerprint.jar",
    );
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let relative_path = "mods/automatic-fingerprint.jar";
    let full_path = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"automatic fingerprint bytes";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let file = stage8_curseforge_file(
        20,
        1001,
        "automatic-fingerprint.jar",
        bytes.len() as u64,
        Vec::new(),
        compute_fingerprint(bytes) as u64,
    );

    record_installed_curseforge_file_with_permit(
        &instance_id,
        relative_path,
        &full_path,
        &file,
        ProjectType::Mod,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        None,
        &state,
    )
    .await
    .unwrap();

    assert!(stage8_pending_keys(&state, &instance_id).await.is_empty());
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_automatic_weak_record_keeps_pending() {
    let (state, instance_id) =
        create_stage6_instance("automatic weak metadata").await;
    let download =
        stage8_legacy_manual_download(21, 1101, "automatic-md5.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let relative_path = "mods/automatic-md5.jar";
    let full_path = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"automatic MD5-only bytes";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let file = stage8_curseforge_file(
        21,
        1101,
        "automatic-md5.jar",
        bytes.len() as u64,
        vec![CurseForgeFileHash {
            value: "validated-md5".to_string(),
            algo: 2,
        }],
        0,
    );

    record_installed_curseforge_file_with_permit(
        &instance_id,
        relative_path,
        &full_path,
        &file,
        ProjectType::Mod,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        None,
        &state,
    )
    .await
    .unwrap();

    assert!(
        stage8_pending_keys(&state, &instance_id)
            .await
            .contains(&("21".to_string(), "1101".to_string()))
    );
    let snapshot = crate::api::instance::get_content_snapshot(&instance_id)
        .await
        .unwrap();
    assert!(snapshot.items.iter().any(|item| {
        item.provider_project_id.as_deref() == Some("21")
            && item.provider_release_id.as_deref() == Some("1101")
            && item.materialization_state
                == crate::state::instances::PackMemberMaterializationState::Present
    }));
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_rollback_identity_record_does_not_complete_pending() {
    let (state, instance_id) =
        create_stage6_instance("rollback provenance").await;
    let download = stage8_legacy_manual_download(22, 1201, "rollback.jar");
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let relative_path = "mods/rollback.jar";
    let full_path = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join(relative_path);
    let bytes = b"historical rollback bytes";
    crate::util::io::write(&full_path, bytes).await.unwrap();
    let (_, sha1) = sha1_file_async(&full_path).await.unwrap();
    let provider_ref = ContentProviderRef::CurseForge {
        project_id: CurseForgeProjectId::new(22).unwrap(),
        file_id: Some(CurseForgeFileId::new(1201).unwrap()),
    };

    crate::state::record_project_file_atomic(
        &instance_id,
        relative_path,
        &sha1,
        bytes.len() as u64,
        ProjectType::Mod,
        ContentSourceKind::CurseForge,
        crate::state::instances::ContentOwnershipKind::PackManaged,
        Some(&provider_ref),
        true,
        None,
        &state,
    )
    .await
    .unwrap();

    assert!(
        stage8_pending_keys(&state, &instance_id)
            .await
            .contains(&("22".to_string(), "1201".to_string()))
    );
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_non_pending_import_fails_before_verification() {
    let (state, instance_id) =
        create_stage6_instance("non-pending import").await;
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join("invalid.jar");
    crate::util::io::write(&source, b"not the required file")
        .await
        .unwrap();

    let error = import_pending_manual_download_file(
        &instance_id,
        601,
        6001,
        source.clone(),
    )
    .await
    .expect_err("missing pending identity must be rejected");

    assert!(matches!(
        error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "The selected CurseForge file is not pending for this instance"
    ));
    assert!(source.exists());
    assert!(
        crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
            &instance_id,
            &state.pool,
        )
        .await
        .unwrap()
        .is_empty()
    );
    let target = crate::api::instance::get_full_path(&instance_id)
        .await
        .unwrap()
        .join("mods/invalid.jar");
    assert!(!target.exists());
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn curseforge_wrong_pending_file_fails_integrity() {
    let (state, instance_id) =
        create_stage6_instance("wrong pending file").await;
    let download = stage6_manual_download(
        701,
        7001,
        "expected.jar",
        b"required pending bytes",
    );
    persist_manual_download(&instance_id, &download)
        .await
        .unwrap();
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join("selected.jar");
    crate::util::io::write(&source, b"wrong pending bytes")
        .await
        .unwrap();

    let error = import_pending_manual_download_file(
        &instance_id,
        download.project_id,
        download.file_id,
        source.clone(),
    )
    .await
    .expect_err("wrong pending file must fail integrity");

    assert!(matches!(
        error.raw.as_ref(),
        ErrorKind::InputError(message)
            if message
                == "The selected file does not match the required CurseForge file"
    ));
    assert!(source.exists());
    let pending = crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
        &instance_id,
        &state.pool,
    )
    .await
    .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provider_project_id, "701");
    assert_eq!(pending[0].provider_release_id, "7001");
    let snapshot = crate::api::instance::get_content_snapshot(&instance_id)
        .await
        .unwrap();
    assert!(!snapshot.items.iter().any(|item| {
        item.provider_project_id.as_deref() == Some("701")
            && item.provider_release_id.as_deref() == Some("7001")
            && item.materialization_state
                == crate::state::instances::PackMemberMaterializationState::Present
            && item.content.is_some()
    }));
    crate::api::instance::remove(&instance_id).await.unwrap();
}

#[cfg(not(feature = "tauri"))]
#[tokio::test]
async fn manual_import_completes_persisted_pack_member() {
    crate::event::EventState::init().await.unwrap();
    let state_root = tempfile::tempdir().unwrap().keep();
    let state =
        State::init_for_test(state_root.to_string_lossy().to_string())
            .await
            .unwrap();
    let created = crate::api::instance::create(
        format!("CurseForge drag {}", uuid::Uuid::new_v4()),
        "1.12.2".to_string(),
        ModLoader::Forge,
        Some("14.23.5.2860".to_string()),
        None,
        InstanceLink::Unmanaged,
        None,
        None,
    )
    .await
    .unwrap();
    let source_directory = tempfile::tempdir().unwrap();
    let source = source_directory.path().join("example-mod.jar");
    let bytes = b"verified curseforge drag";
    crate::util::io::write(&source, bytes).await.unwrap();
    let (_, sha1) = sha1_file_async(&source).await.unwrap();
    let download = CurseForgeManualDownload {
        project_id: 123,
        file_id: 456,
        file_name: "example-mod.jar".to_string(),
        ownership_kind:
            crate::state::instances::ContentOwnershipKind::PackManaged,
        operation_kind:
            crate::state::instances::ManualDownloadOperationKind::PackInstall,
        website_url: None,
        project_type: "mod".to_string(),
        project_slug: "example-mod".to_string(),
        target_folder: "mods".to_string(),
        hashes: vec![CurseForgeFileHash {
            value: sha1,
            algo: 1,
        }],
        file_length: bytes.len() as u64,
        file_fingerprint: 0,
    };
    persist_manual_download(&created.instance.id, &download)
        .await
        .unwrap();

    crate::util::io::write(&source, b"same-length-wrong-bytes!")
        .await
        .unwrap();
    assert!(
        import_pending_manual_download_from_path(
            &created.instance.id,
            &source,
        )
        .await
        .is_err()
    );
    assert_eq!(
        crate::state::instances::adapters::sqlite::content_rows::get_pending_manual_downloads(
            &created.instance.id,
            &state.pool,
        )
        .await
        .unwrap()
        .len(),
        1
    );
    let selected = source_directory.path().join("renamed-local-file.jar");
    crate::util::io::write(&selected, bytes).await.unwrap();

    let relative_path = import_pending_manual_download_file(
        &created.instance.id,
        123,
        456,
        selected.clone(),
    )
    .await
    .unwrap()
    .relative_path;
    let snapshot =
        crate::api::instance::get_content_snapshot(&created.instance.id)
            .await
            .unwrap();
    let imported = snapshot
        .items
        .iter()
        .find(|item| {
            item.provider_project_id.as_deref() == Some("123")
                && item.provider_release_id.as_deref() == Some("456")
        })
        .unwrap();

    assert!(snapshot.pending_manual_downloads.is_empty());
    assert_eq!(
        imported.ownership_kind,
        crate::state::instances::ContentOwnershipKind::PackManaged
    );
    assert_eq!(
        imported.materialization_state,
        crate::state::instances::PackMemberMaterializationState::Present
    );
    assert_eq!(imported.expected_relative_path, relative_path);
    let instance_path = state
        .directories
        .instance_game_dir(&created.instance)
        .join(relative_path);
    assert_eq!(crate::util::io::read(&instance_path).await.unwrap(), bytes);
    assert!(source.exists());
    assert!(selected.exists());
    crate::util::io::write(&selected, b"mutated selected source")
        .await
        .unwrap();
    assert_eq!(crate::util::io::read(instance_path).await.unwrap(), bytes);
}

#[test]
fn archive_paths_stay_inside_the_instance() {
    assert_eq!(
        safe_archive_relative_path("config/example.toml").unwrap(),
        "config/example.toml"
    );
    assert!(safe_archive_relative_path("../options.txt").is_err());
    assert!(safe_archive_relative_path("/options.txt").is_err());
}

#[test]
fn curseforge_override_content_joins_the_pack_group() {
    assert_eq!(
        curseforge_override_content("mods/cc-tweaked.jar").unwrap(),
        Some(CurseForgePackExpectedOverride {
            project_type: ProjectType::Mod,
            expected_relative_path: "mods/cc-tweaked.jar".to_string(),
        })
    );
    assert_eq!(
        curseforge_override_content("datapacks/sawmill.zip").unwrap(),
        Some(CurseForgePackExpectedOverride {
            project_type: ProjectType::DataPack,
            expected_relative_path: "datapacks/sawmill.zip".to_string(),
        })
    );
    assert_eq!(
        curseforge_override_content("config/example.toml").unwrap(),
        None
    );
}

#[test]
fn cdn_urls_are_restricted_to_forgecdn() {
    assert!(
        validate_cdn_url(
            &reqwest::Url::parse(
                "https://edge.forgecdn.net/files/1/2/a.jar"
            )
            .unwrap()
        )
        .is_ok()
    );
    assert!(
        validate_cdn_url(
            &reqwest::Url::parse("https://forgecdn.net.evil.test/a.jar")
                .unwrap()
        )
        .is_err()
    );
}

#[test]
fn blank_download_urls_are_treated_as_unavailable() {
    assert_eq!(normalized_download_url(None), None);
    assert_eq!(normalized_download_url(Some(String::new())), None);
    assert_eq!(normalized_download_url(Some("  \t".to_string())), None);
    assert_eq!(
        normalized_download_url(Some(
            " https://edge.forgecdn.net/files/1/2/a.jar ".to_string(),
        )),
        Some("https://edge.forgecdn.net/files/1/2/a.jar".to_string()),
    );
}

#[test]
fn derived_download_urls_follow_curseforge_file_id_layout() {
    assert_eq!(
        derived_curseforge_download_url(
            6_767_951,
            "jei 1.21.5+neoforge.jar"
        )
        .unwrap(),
        "https://edge.forgecdn.net/files/6767/951/jei%201.21.5%2Bneoforge.jar"
    );
    assert_eq!(
        derived_curseforge_download_url(1_234_005, "example.jar").unwrap(),
        "https://edge.forgecdn.net/files/1234/5/example.jar"
    );
    assert!(
        derived_curseforge_download_url(1_234_005, "../example.jar")
            .is_err()
    );
}

#[test]
fn curseforge_loader_ids_map_to_instance_loaders() {
    assert_eq!(loader_family("forge-47.4.0"), "forge");
    assert_eq!(loader_family("fabric-0.16.10"), "fabric");
    assert_eq!(loader_type("cleanroom"), Some(1));
    assert_eq!(loader_type("Fabric"), Some(4));
    assert_eq!(loader_type("neoforge"), Some(6));
}

#[test]
fn curseforge_game_versions_ignore_loader_labels() {
    assert!(looks_like_game_version("1.21.8"));
    assert!(!looks_like_game_version("Fabric"));
    assert!(!looks_like_game_version("forge"));
}

#[test]
fn modpack_target_uses_primary_forge_loader_and_version() {
    let manifest = CurseForgeModpackManifest {
        minecraft: CurseForgeManifestMinecraft {
            version: "1.12.2".to_string(),
            mod_loaders: vec![CurseForgeManifestLoader {
                id: "forge-14.23.5.2860".to_string(),
                primary: true,
            }],
        },
        files: Vec::new(),
        overrides: "overrides".to_string(),
        name: None,
        version: None,
    };

    assert_eq!(
        modpack_target(&manifest).unwrap(),
        CurseForgeModpackTarget {
            game_version: "1.12.2".to_string(),
            loader: ModLoader::Forge,
            loader_version: Some("14.23.5.2860".to_string()),
        }
    );
}

#[test]
fn modpack_target_recognizes_pcl_cleanroom_version_convention() {
    let manifest = CurseForgeModpackManifest {
        minecraft: CurseForgeManifestMinecraft {
            version: "1.12.2".to_string(),
            mod_loaders: vec![CurseForgeManifestLoader {
                id: "forge-0.3.0-alpha".to_string(),
                primary: true,
            }],
        },
        files: Vec::new(),
        overrides: "overrides".to_string(),
        name: None,
        version: None,
    };

    assert_eq!(
        modpack_target(&manifest).unwrap(),
        CurseForgeModpackTarget {
            game_version: "1.12.2".to_string(),
            loader: ModLoader::Cleanroom,
            loader_version: Some("0.3.0-alpha".to_string()),
        }
    );
}

#[test]
fn modpack_target_without_loader_is_vanilla() {
    let manifest = CurseForgeModpackManifest {
        minecraft: CurseForgeManifestMinecraft {
            version: "1.20.1".to_string(),
            mod_loaders: Vec::new(),
        },
        files: Vec::new(),
        overrides: "overrides".to_string(),
        name: None,
        version: None,
    };

    assert_eq!(
        modpack_target(&manifest).unwrap(),
        CurseForgeModpackTarget {
            game_version: "1.20.1".to_string(),
            loader: ModLoader::Vanilla,
            loader_version: None,
        }
    );
}

#[test]
fn recognized_project_types_map_curseforge_classes() {
    assert_eq!(recognized_project_type(Some(6)), Some(ProjectType::Mod));
    assert_eq!(
        recognized_project_type(Some(12)),
        Some(ProjectType::ResourcePack),
    );
    assert_eq!(
        recognized_project_type(Some(6552)),
        Some(ProjectType::ShaderPack),
    );
    assert_eq!(
        recognized_project_type(Some(6945)),
        Some(ProjectType::DataPack),
    );
    assert_eq!(
        recognized_project_type(Some(17)),
        Some(ProjectType::WorldSave),
    );
    assert_eq!(recognized_project_type(Some(4471)), None);
    assert_eq!(recognized_project_type(None), None);
}

#[test]
fn world_archives_must_use_a_zip_file_name() {
    assert!(validate_world_archive_name("world.zip").is_ok());
    assert!(validate_world_archive_name("WORLD.ZIP").is_ok());
    assert!(validate_world_archive_name("world.rar").is_err());
    assert!(validate_world_archive_name("../world.zip").is_err());
}

#[test]
fn a_key_set_in_the_launcher_wins_over_the_launcher_key() {
    assert_eq!(
        resolve_api_key(Some("override".into()), Some("launcher".into())),
        (
            Some("override".to_string()),
            CurseForgeApiKeySource::Override
        ),
    );
}

#[test]
fn a_blank_override_falls_back_to_the_launcher_key() {
    assert_eq!(
        resolve_api_key(Some("   ".into()), Some("launcher".into())),
        (
            Some("launcher".to_string()),
            CurseForgeApiKeySource::Launcher
        ),
    );
    assert_eq!(
        resolve_api_key(None, Some("launcher".into())),
        (
            Some("launcher".to_string()),
            CurseForgeApiKeySource::Launcher
        ),
    );
}

#[test]
fn keys_are_trimmed_before_they_are_used() {
    assert_eq!(
        resolve_api_key(Some(" override\n".into()), None),
        (
            Some("override".to_string()),
            CurseForgeApiKeySource::Override
        ),
    );
}

#[test]
fn a_build_without_any_key_reports_no_key() {
    assert_eq!(
        resolve_api_key(None, None),
        (None, CurseForgeApiKeySource::Missing),
    );
    assert_eq!(
        resolve_api_key(Some(" ".into()), Some("\t".into())),
        (None, CurseForgeApiKeySource::Missing),
    );
}