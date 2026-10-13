use super::*;
use crate::state::{DirectoryInfo, ModrinthProjectId, ModrinthVersionId};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

fn artifact(
    source: SourceId,
    project: &str,
    release: &str,
    bytes: &[u8],
) -> ResourceArtifact {
    ResourceArtifact {
        reference: SourceArtifactRef {
            release: SourceReleaseRef {
                project: SourceProjectRef {
                    source,
                    project_id: SourceProjectId::new(project).unwrap(),
                },
                release_id: SourceReleaseId::new(release).unwrap(),
            },
            artifact_id: SourceArtifactId::new(release).unwrap(),
        },
        file_name: Some(format!("{release}.zip")),
        role: ArtifactRole::Primary,
        available: Some(true),
        integrity: ArtifactIntegrity {
            size: Some(bytes.len() as u64),
            hashes: vec![ContentHash {
                algorithm: HashAlgorithm::Sha1,
                value: sha1_smol::Sha1::from(bytes).hexdigest(),
            }],
        },
        acquisition: ArtifactAcquisition::PublicDownload {
            urls: vec!["https://invalid.example/frozen.zip".into()],
        },
        dependencies: vec![],
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    std::sync::Arc<crate::State>,
    InstalledResource,
    ResourceArtifact,
    PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let directories = DirectoryInfo {
        settings_dir: temp.path().to_path_buf(),
        config_dir: temp.path().to_path_buf(),
        app_identifier: "resource-execution-test".into(),
    };
    let root = directories.instances_dir().join("example");
    std::fs::create_dir_all(root.join("resourcepacks")).unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(temp.path().join("state.db"))
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO instances(id, path, install_stage, launcher_feature_version, name, created, modified) VALUES ('instance', 'example', 'installed', '1', 'Example', 1, 1); INSERT INTO instance_content_sets(id, instance_id, name, source_kind, status, game_version, loader, created, modified) VALUES ('set', 'instance', 'Example', 'local', 'available', '1.20.1', 'fabric', 1, 1); UPDATE instances SET applied_content_set_id = 'set' WHERE id = 'instance';").execute(&pool).await.unwrap();
    let state = crate::state::test_state(directories, pool.clone())
        .await
        .unwrap();
    let old = artifact(SourceId::modrinth(), "root0000", "old", b"old bytes");
    let current_path = "resourcepacks/prefix-old.zip.disabled";
    std::fs::write(root.join(current_path), b"old bytes").unwrap();
    let file = content_rows::upsert_instance_file_from_parts(
        content_rows::UpsertInstanceFile {
            instance_id: "instance",
            relative_path: current_path,
            file_name: "prefix-old.zip.disabled",
            enabled: false,
            sha1: &old.integrity.hashes[0].value,
            size: 9,
            missing: false,
            local_mod_data: None,
            icon_path: None,
        },
        &pool,
    )
    .await
    .unwrap();
    let entry = content_rows::upsert_content_entry_from_parts(
        content_rows::UpsertContentEntry {
            instance_id: "instance",
            content_set_id: "set",
            file_id: Some(&file.id),
            project_type: ProjectType::ResourcePack,
            source_kind: ContentSourceKind::Local,
            ownership_kind: ContentOwnershipKind::UserAdded,
            auto_dependency: false,
            server_requirement: ContentRequirement::Optional,
            client_requirement: ContentRequirement::Required,
            enabled: false,
        },
        &pool,
    )
    .await
    .unwrap();
    let reference = ContentProviderRef::Modrinth {
        project_id: ModrinthProjectId::new("root0000").unwrap(),
        version_id: Some(ModrinthVersionId::new("old").unwrap()),
    };
    content_rows::upsert_content_provider_ref(
        &entry.id, &reference, true, &pool,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO resource_installed_artifacts(entry_id, artifact, scope) VALUES (?, ?, ?)").bind(&entry.id).bind(serde_json::to_string(&old).unwrap()).bind(serde_json::to_string(&InstallScope::Instance).unwrap()).execute(&pool).await.unwrap();
    let installed =
        plans::installed(&pool, "instance").await.unwrap().remove(0);
    (temp, state, installed, old, root)
}

async fn local_plan(
    state: &crate::State,
    current: InstalledResource,
    target: ResourceArtifact,
    source_path: &str,
    restore: Option<HistoryId>,
) -> ResourceOperationPlan {
    let id = PlanId::new(uuid::Uuid::new_v4().to_string()).unwrap();
    let node_id = SelectionId::new("root").unwrap();
    let reference = ResourceRef::Source {
        project: target.reference.release.project.clone(),
    };
    let plan = ResourceOperationPlan {
        schema_version: ResourceContractVersion,
        id: id.clone(),
        revision: 1,
        instance_id: "instance".into(),
        instance_revision: plans::instance_revision(&state.pool, "instance")
            .await
            .unwrap(),
        intent: ResourceOperationIntent::SwitchVersion,
        selections: vec![ResourceSelection {
            id: node_id.clone(),
            resource: reference.clone(),
            release: ReleaseSelection::Artifact {
                reference: target.reference.clone(),
            },
            resource_type: ResourceType::ResourcePack,
            scope: current.scope.clone(),
            entry_id: Some(current.entry_id.clone()),
        }],
        active_roots: vec![node_id.clone()],
        nodes: vec![ResourceDependencyNode {
            id: node_id.clone(),
            root_selection_ids: vec![node_id.clone()],
            resource_type: Some(ResourceType::ResourcePack),
            resource: reference,
            scope: current.scope.clone(),
            state: DependencyNodeState::Update,
            artifact: Some(target.clone()),
            installed_entries: vec![],
            compatibility: CompatibilityAssessment {
                game_version: CompatibilityState::Compatible,
                loader: CompatibilityState::Compatible,
                reasons: vec![],
            },
            declared_compatibility: Some(ReleaseCompatibility {
                game_versions: vec!["1.20.1".into()],
                loaders: vec![],
            }),
        }],
        edges: vec![],
        actions: vec![ResourceEntryAction {
            entry_id: current.entry_id.clone(),
            node_id,
            kind: ResourceActionKind::Replace,
            target,
            target_relative_path: current.relative_path.clone(),
            current: Some(current.clone()),
        }],
        issues: vec![],
        exceptions: vec![],
        selection_reasons: vec![],
    };
    let draft = plans::OperationDraft {
        naming: Default::default(),
        plan: plan.clone(),
        install_dependencies: false,
        candidates: vec![],
        local_files: HashMap::from([(
            current.entry_id.clone(),
            source_path.into(),
        )]),
        restore_history: restore,
    };
    sqlx::query("INSERT INTO resource_operation_plans(id, instance_id, revision, payload) VALUES (?, 'instance', 1, ?)").bind(id.as_str()).bind(serde_json::to_string(&draft).unwrap()).execute(&state.pool).await.unwrap();
    plan
}

#[tokio::test]
async fn exact_cross_source_replace_and_rollback_preserve_entry_disabled_path_and_metadata()
 {
    let (_temp, state, original, old, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let mut target =
        artifact(SourceId::curseforge(), "123", "456", b"new bytes");
    use sha2::Digest;
    target.integrity.hashes = vec![ContentHash {
        algorithm: HashAlgorithm::Sha512,
        value: format!("{:x}", sha2::Sha512::digest(b"new bytes")),
    }];
    let plan =
        local_plan(&state, original.clone(), target.clone(), "new.zip", None)
            .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        execute("instance", &plan, None, &state),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"new bytes"
    );
    let changed = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(changed.entry_id, original.entry_id);
    assert_eq!(changed.relative_path, original.relative_path);
    assert!(!changed.enabled);
    assert_eq!(changed.artifact, Some(target.reference));
    let mut snapshot = vec![crate::state::InstanceContentSnapshotItem {
        compatibility: None,
        resource_id: None,
        pack_resource_id: None,
        file_id: None,
        entry_id: Some(original.entry_id.as_str().into()),
        member_id: None,
        ownership_kind: ContentOwnershipKind::UserAdded,
        materialization_state: crate::state::PackMemberMaterializationState::Present,
        override_kind: crate::state::PackMemberOverrideKind::None,
        expected_relative_path: original.relative_path.clone(),
        required: false,
        project_type: ProjectType::ResourcePack,
        provider: None,
        provider_project_id: None,
        provider_release_id: None,
        content: Some(crate::state::ContentItem {
            file_name: "prefix-old.zip.disabled".into(),
            ..serde_json::from_value(serde_json::json!({
                "file_name": "prefix-old.zip.disabled", "project_type": "resourcepack",
                "file_path": original.relative_path, "id": "test", "size": 9,
                "enabled": false, "provider_refs": [], "external": false
            })).unwrap()
        }),
        capabilities: Default::default(),
        dependency: None,
    }];
    history::attach_to_snapshot("instance", &mut snapshot, &state)
        .await
        .unwrap();
    super::installed::attach_compatibility(
        &state.pool,
        "instance",
        "1.21.1",
        "neoforge",
        &mut snapshot,
    )
    .await
    .unwrap();
    assert_eq!(
        snapshot[0].compatibility.as_ref().unwrap().game_version,
        CompatibilityState::Incompatible
    );
    assert_eq!(
        snapshot[0].compatibility.as_ref().unwrap().loader,
        CompatibilityState::Compatible
    );
    super::installed::attach_compatibility(
        &state.pool,
        "instance",
        "1.20.1",
        "fabric",
        &mut snapshot,
    )
    .await
    .unwrap();
    assert_eq!(
        snapshot[0].compatibility.as_ref().unwrap().game_version,
        CompatibilityState::Compatible
    );
    let saved: String = sqlx::query_scalar(
        "SELECT artifact FROM resource_installed_artifacts WHERE entry_id = ?",
    )
    .bind(original.entry_id.as_str())
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let record: super::installed::InstalledArtifactRecord =
        serde_json::from_str(&saved).unwrap();
    assert_eq!(
        record
            .declared_compatibility
            .as_ref()
            .unwrap()
            .game_versions,
        vec!["1.20.1"]
    );
    let legacy: ResourceArtifact = serde_json::from_str(&saved).unwrap();
    assert_eq!(legacy.reference, record.artifact.reference);
    let historical: super::installed::InstalledArtifactRecord =
        serde_json::from_str(&serde_json::to_string(&legacy).unwrap()).unwrap();
    assert!(historical.declared_compatibility.is_none());
    assert!(snapshot[0].content.as_ref().unwrap().rollback.is_some());
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM instance_content_entries")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let history_json: String = sqlx::query_scalar(
        "SELECT payload FROM resource_history WHERE entry_id = ?",
    )
    .bind(original.entry_id.as_str())
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let history: ResourceHistoryEntry =
        serde_json::from_str(&history_json).unwrap();
    assert!(
        matches_file(
            &root.join(history.backup_relative_path.as_ref().unwrap()),
            &original.integrity
        )
        .await
        .unwrap()
    );
    assert!(serde_json::from_str::<serde_json::Value>(&history_json).unwrap()["previousArtifact"].is_object());
    execute("instance", &plan, None, &state).await.unwrap();
    let request = crate::install::InstallRequest::InstallContentBatch {
        instance_id: "instance".into(),
        items: vec![crate::install::InstallContentBatchItem::FrozenResource {
            plan: plan.clone(),
        }],
        display_title: "Test".into(),
        display_icon: None,
    };
    assert!(
        plans::frozen_request_completed(&request, &state.pool)
            .await
            .unwrap()
    );
    let mut job = crate::install::model::InstallJobState::new(request);
    job.record_event(crate::install::InstallJobEventKind::JobSucceeded {
        instance_id: Some("instance".into()),
    });
    let job_id = uuid::Uuid::new_v4();
    crate::install::store::insert(
        job_id,
        &job,
        crate::install::InstallJobStatus::Canceling,
        &state,
    )
    .await
    .unwrap();
    let completed =
        crate::install::store::complete_running_job(job_id, &job, &state)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        completed.status,
        crate::install::InstallJobStatus::Succeeded
    );
    let restart_id = uuid::Uuid::new_v4();
    crate::install::store::insert(
        restart_id,
        &job,
        crate::install::InstallJobStatus::Running,
        &state,
    )
    .await
    .unwrap();
    crate::install::recovery::recover_interrupted_jobs(&state)
        .await
        .unwrap();
    assert_eq!(
        crate::install::store::get_required(restart_id, &state)
            .await
            .unwrap()
            .status,
        crate::install::InstallJobStatus::Succeeded
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM resource_history")
            .fetch_one(&state.pool)
            .await
            .unwrap(),
        1
    );
    let rollback = local_plan(
        &state,
        changed,
        old,
        history.backup_relative_path.as_ref().unwrap(),
        Some(history.id),
    )
    .await;
    execute("instance", &rollback, None, &state).await.unwrap();
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    let restored = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(restored.entry_id, original.entry_id);
    assert_eq!(restored.artifact, original.artifact);
    assert_eq!(restored.ownership, original.ownership);
    assert!(!restored.enabled);
    let entry = content_rows::get_content_entry_by_id(
        original.entry_id.as_str(),
        &state.pool,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(entry.server_requirement, ContentRequirement::Optional);
    assert_eq!(entry.client_requirement, ContentRequirement::Required);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&state.pool)
            .await
            .unwrap()
            .is_empty()
    );
    state.pool.close().await;
}

#[tokio::test]
async fn install_cleanup_restores_resource_journal_without_reselection() {
    let (_temp, state, original, _old, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let target = artifact(SourceId::curseforge(), "123", "456", b"new bytes");
    let plan =
        local_plan(&state, original.clone(), target.clone(), "new.zip", None)
            .await;
    let backup = "resourcepacks/interrupted.old";
    std::fs::rename(root.join(&original.relative_path), root.join(backup))
        .unwrap();
    let journal = vec![JournalFile {
        previous_path: None,
        path: original.relative_path.clone(),
        backup: Some(backup.into()),
        old: Some(original.integrity),
        target: target.integrity,
    }];
    sqlx::query("INSERT INTO resource_execution_journals(plan_id, payload) VALUES (?, ?)").bind(plan.id.as_str()).bind(serde_json::to_string(&journal).unwrap()).execute(&state.pool).await.unwrap();
    let mut job = crate::install::model::InstallJobState::new(
        crate::install::InstallRequest::InstallContentBatch {
            instance_id: "instance".into(),
            items: vec![
                crate::install::InstallContentBatchItem::FrozenResource {
                    plan,
                },
            ],
            display_title: "Recover".into(),
            display_icon: None,
        },
    );
    crate::install::recovery::apply_cleanup(&mut job, &state)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM resource_execution_journals"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    state.pool.close().await;
}

#[tokio::test]
async fn stale_review_and_changed_backup_leave_disk_and_database_untouched() {
    let (_temp, state, original, _old, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let plan = local_plan(
        &state,
        original.clone(),
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    sqlx::query("UPDATE instance_content_sets SET revision = revision + 1 WHERE id = 'set'").execute(&state.pool).await.unwrap();
    assert!(execute("instance", &plan, None, &state).await.is_err());
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM resource_history")
            .fetch_one(&state.pool)
            .await
            .unwrap(),
        0
    );
    let new_plan = local_plan(
        &state,
        original.clone(),
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    std::fs::write(root.join("new.zip"), b"changed!").unwrap();
    assert!(execute("instance", &new_plan, None, &state).await.is_err());
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    state.pool.close().await;
}

#[tokio::test]
async fn interrupted_materialization_recovers_exact_backup_and_rejects_external_changes()
 {
    let temp = tempfile::tempdir().unwrap();
    let old = artifact(SourceId::modrinth(), "root0000", "old", b"old bytes");
    let new = artifact(SourceId::curseforge(), "123", "456", b"new bytes");
    std::fs::write(temp.path().join("backup.old"), b"old bytes").unwrap();
    std::fs::write(temp.path().join("active.zip"), b"new bytes").unwrap();
    let journal = vec![JournalFile {
        previous_path: None,
        path: "active.zip".into(),
        backup: Some("backup.old".into()),
        old: Some(old.integrity),
        target: new.integrity,
    }];
    recover_files(temp.path(), &journal).await.unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("active.zip")).unwrap(),
        b"old bytes"
    );
    std::fs::write(temp.path().join("backup.old"), b"old bytes").unwrap();
    std::fs::write(temp.path().join("active.zip"), b"user changes").unwrap();
    assert!(recover_files(temp.path(), &journal).await.is_err());
    assert_eq!(
        std::fs::read(temp.path().join("active.zip")).unwrap(),
        b"user changes"
    );
}

async fn change_target_path(
    state: &crate::State,
    mut plan: ResourceOperationPlan,
    path: &str,
) -> ResourceOperationPlan {
    plan.actions[0].target_relative_path = path.into();
    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM resource_operation_plans WHERE id = ?",
    )
    .bind(plan.id.as_str())
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let mut draft: plans::OperationDraft =
        serde_json::from_str(&payload).unwrap();
    draft.plan = plan.clone();
    sqlx::query("UPDATE resource_operation_plans SET payload = ? WHERE id = ?")
        .bind(serde_json::to_string(&draft).unwrap())
        .bind(plan.id.as_str())
        .execute(&state.pool)
        .await
        .unwrap();
    plan
}

#[tokio::test]
async fn renamed_update_and_rollback_restore_exact_path_source_and_file_id() {
    let (_temp, state, original, old, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let target = artifact(SourceId::curseforge(), "123", "456", b"new bytes");
    let plan =
        local_plan(&state, original.clone(), target.clone(), "new.zip", None)
            .await;
    let path =
        super::naming::replacement_path(&original, "456.zip", true).unwrap();
    assert_eq!(path, "resourcepacks/prefix-456.zip.disabled");
    let plan = change_target_path(&state, plan, &path).await;
    let original_file: String = sqlx::query_scalar(
        "SELECT file_id FROM instance_content_entries WHERE id = ?",
    )
    .bind(original.entry_id.as_str())
    .fetch_one(&state.pool)
    .await
    .unwrap();
    execute("instance", &plan, None, &state).await.unwrap();
    assert!(!root.join(&original.relative_path).exists());
    assert_eq!(std::fs::read(root.join(&path)).unwrap(), b"new bytes");
    let changed = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(changed.relative_path, path);
    assert_eq!(changed.original_file_name.as_deref(), Some("456.zip"));
    let histories =
        history::list_with_state("instance", Some(&original.entry_id), &state)
            .await
            .unwrap();
    let entry = &histories[0];
    assert_eq!(entry.previous.relative_path, original.relative_path);
    assert_eq!(entry.target.relative_path, path);
    let draft = history::prepare_rollback(
        "instance".into(),
        changed.entry_id,
        entry.id.clone(),
        &state,
    )
    .await
    .unwrap();
    let rollback = draft.plan.clone();
    assert_eq!(rollback.actions[0].target.reference, old.reference);
    sqlx::query("INSERT INTO resource_operation_plans(id, instance_id, revision, payload) VALUES (?, 'instance', 1, ?)").bind(rollback.id.as_str()).bind(serde_json::to_string(&draft).unwrap()).execute(&state.pool).await.unwrap();
    execute("instance", &rollback, None, &state).await.unwrap();
    assert!(!root.join(&path).exists());
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    let restored = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(restored, original);
    let file: String = sqlx::query_scalar(
        "SELECT file_id FROM instance_content_entries WHERE id = ?",
    )
    .bind(original.entry_id.as_str())
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(file, original_file);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&state.pool)
            .await
            .unwrap()
            .is_empty()
    );
    state.pool.close().await;
}

#[tokio::test]
async fn renamed_update_collision_and_sql_failure_preserve_original_bytes_and_identity()
 {
    let (_temp, state, original, _, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let plan = local_plan(
        &state,
        original.clone(),
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    let destination = "resourcepacks/new.zip.disabled";
    let plan = change_target_path(&state, plan, destination).await;
    std::fs::write(root.join(destination), b"user file").unwrap();
    assert!(execute("instance", &plan, None, &state).await.is_err());
    assert_eq!(std::fs::read(root.join(destination)).unwrap(), b"user file");
    std::fs::remove_file(root.join(destination)).unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_resource_write BEFORE INSERT ON resource_installed_artifacts BEGIN SELECT RAISE(ABORT, 'test failure'); END;").execute(&state.pool).await.unwrap();
    assert!(execute("instance", &plan, None, &state).await.is_err());
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    assert!(!root.join(destination).exists());
    assert_eq!(
        plans::installed(&state.pool, "instance").await.unwrap()[0],
        original
    );
    state.pool.close().await;
}

#[tokio::test]
async fn backup_cleanup_checks_selection_snapshot_tasks_references_and_rollback_loss()
 {
    let (_temp, state, original, _, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let plan = local_plan(
        &state,
        original.clone(),
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    execute("instance", &plan, None, &state).await.unwrap();
    std::fs::write(root.join("resourcepacks/unknown.old"), b"unknown").unwrap();
    std::fs::write(root.join("resourcepacks/new.zip_old.zip.old"), b"orphan")
        .unwrap();
    let files = super::history_cleanup::scan_with_state("instance", &state)
        .await
        .unwrap();
    assert_eq!(files.len(), 3);
    let valid = files.iter().find(|file| file.rollback_available).unwrap();
    let orphan = files
        .iter()
        .find(|file| file.history_id.is_none() && file.recognized)
        .unwrap();
    let unknown = files.iter().find(|file| !file.recognized).unwrap();
    assert!(
        super::history_cleanup::prepare_with_state(
            "instance",
            vec![unknown.id.clone()],
            &state
        )
        .await
        .is_err()
    );
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![valid.id.clone(), orphan.id.clone()],
        &state,
    )
    .await
    .unwrap();
    assert!(root.join(&valid.relative_path).is_file());
    sqlx::query("UPDATE instance_content_sets SET revision = revision + 1 WHERE id = 'set'").execute(&state.pool).await.unwrap();
    assert!(
        super::history_cleanup::execute_with_state(
            review.id,
            review.revision,
            &state
        )
        .await
        .is_err()
    );
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![valid.id.clone(), orphan.id.clone()],
        &state,
    )
    .await
    .unwrap();
    std::fs::write(root.join(&orphan.relative_path), b"modified").unwrap();
    assert!(
        super::history_cleanup::execute_with_state(
            review.id,
            review.revision,
            &state
        )
        .await
        .is_err()
    );
    assert!(root.join(&valid.relative_path).exists());
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![valid.id.clone(), orphan.id.clone()],
        &state,
    )
    .await
    .unwrap();
    let job = crate::install::model::InstallJobState::new(
        crate::install::InstallRequest::InstallContentBatch {
            instance_id: "instance".into(),
            items: vec![],
            display_title: "Busy".into(),
            display_icon: None,
        },
    );
    let id = uuid::Uuid::new_v4();
    crate::install::store::insert(
        id,
        &job,
        crate::install::InstallJobStatus::Queued,
        &state,
    )
    .await
    .unwrap();
    assert!(
        super::history_cleanup::execute_with_state(
            review.id.clone(),
            review.revision,
            &state
        )
        .await
        .is_err()
    );
    sqlx::query("DELETE FROM install_jobs WHERE id = ?")
        .bind(id.to_string())
        .execute(&state.pool)
        .await
        .unwrap();
    let result = super::history_cleanup::execute_with_state(
        review.id.clone(),
        review.revision,
        &state,
    )
    .await
    .unwrap();
    assert_eq!(result.removed.len(), 2);
    assert!(result.failures.is_empty());
    assert!(
        super::history_cleanup::execute_with_state(
            review.id,
            review.revision,
            &state
        )
        .await
        .is_err()
    );
    assert!(!root.join(&valid.relative_path).exists());
    assert!(!root.join(&orphan.relative_path).exists());
    assert!(root.join(&unknown.relative_path).exists());
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"new bytes"
    );
    assert!(
        !history::list_with_state("instance", Some(&original.entry_id), &state)
            .await
            .unwrap()[0]
            .backup_available
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM store_retained_refs WHERE reason = ?",
    )
    .bind(format!("resource-history:{}", valid.id.as_str()))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(retained, 0);
    state.pool.close().await;
}

#[tokio::test]
async fn naming_preferences_cover_unknown_ambiguous_custom_disabled_names() {
    let (_temp, state, mut current, old, _) = fixture().await;
    for (path, original, preserve, expected) in [
        (
            "resourcepacks/[用途] prefix-old.zip suffix.zip.disabled",
            Some("old.zip"),
            true,
            "resourcepacks/[用途] prefix-new.zip suffix.zip.disabled",
        ),
        (
            "resourcepacks/old.zip-old.zip.disabled",
            Some("old.zip"),
            true,
            "resourcepacks/old.zip-old.zip.disabled",
        ),
        (
            "resourcepacks/custom.zip",
            None,
            true,
            "resourcepacks/custom.zip",
        ),
        (
            "resourcepacks/custom.zip.disabled",
            Some("old.zip"),
            true,
            "resourcepacks/custom.zip.disabled",
        ),
        (
            "resourcepacks/prefix-old.zip.disabled",
            Some("old.zip"),
            false,
            "resourcepacks/new.zip.disabled",
        ),
    ] {
        current.relative_path = path.into();
        current.original_file_name = original.map(str::to_owned);
        assert_eq!(
            super::naming::replacement_path(&current, "new.zip", preserve)
                .unwrap(),
            expected
        );
    }
    let store = crate::state::resources::ResourceStore::new(state.pool.clone());
    store
        .add_alias(&old.reference.release.project, "ae2")
        .await
        .unwrap();
    let service = state.resources().await.unwrap();
    let path = service
        .named_path("resourcepacks/old.zip".into(), &old, &Default::default())
        .await
        .unwrap();
    let title =
        crate::api::content_search::chinese_file_title_for_modrinth_slug("ae2")
            .unwrap();
    assert_eq!(path, format!("resourcepacks/[{title}]old.zip"));
    let prefs = super::naming::NamingPreferences {
        chinese: false,
        preserve: true,
    };
    assert_eq!(
        service
            .named_path("resourcepacks/old.zip".into(), &old, &prefs)
            .await
            .unwrap(),
        "resourcepacks/old.zip"
    );
    state.pool.close().await;
}

async fn pack_fixture(state: &crate::State, entry: &InstalledResource) {
    sqlx::query("UPDATE instance_content_entries SET ownership_kind = 'pack_managed', source_kind = 'imported_modpack' WHERE id = ?").bind(entry.entry_id.as_str()).execute(&state.pool).await.unwrap();
    sqlx::query("INSERT INTO instance_pack_members(id, content_set_id, content_entry_id, member_key, project_type, expected_relative_path, provider, provider_project_id, provider_release_id, created_at, modified_at) VALUES ('member', 'set', ?, 'member', 'resourcepack', ?, 'modrinth', 'root0000', 'old', 1, 1)").bind(entry.entry_id.as_str()).bind(&entry.relative_path).execute(&state.pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO instance_pack_members(id, content_set_id, member_key, project_type, expected_relative_path, materialization_state, created_at, modified_at) VALUES ('missing', 'set', 'missing', 'mod', 'mods/missing.jar', 'missing', 1, 1);").execute(&state.pool).await.unwrap();
}

#[tokio::test]
async fn explicit_unlink_preserves_files_sources_and_repairs_only_recorded_unlinks()
 {
    let (_temp, state, original, _, root) = fixture().await;
    pack_fixture(&state, &original).await;
    crate::state::instances::commands::repair_recorded_pack_unlink(
        "instance",
        &state.pool,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        2
    );
    let patch = crate::state::EditInstance {
        link: Some(crate::state::InstanceLink::Unmanaged),
        ..Default::default()
    };
    crate::state::edit_instance("instance", patch, &state.pool)
        .await
        .unwrap();
    assert!(root.join(&original.relative_path).exists());
    let installed = plans::installed(&state.pool, "instance").await.unwrap();
    assert_eq!(installed[0].artifact, original.artifact);
    assert_eq!(installed[0].ownership, ResourceOwnership::UserAdded);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    pack_fixture(&state, &original).await;
    crate::state::instances::commands::repair_recorded_pack_unlink(
        "instance",
        &state.pool,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    std::fs::remove_file(root.join(&original.relative_path)).unwrap();
    crate::state::instances::commands::repair_recorded_pack_unlink(
        "instance",
        &state.pool,
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&state.pool)
            .await
            .unwrap()
            .is_empty()
    );
    state.pool.close().await;
}

#[tokio::test]
async fn unlink_task_conflict_and_sql_failure_leave_pack_metadata_intact() {
    let (_temp, state, original, _, _) = fixture().await;
    pack_fixture(&state, &original).await;
    let patch = crate::state::EditInstance {
        link: Some(crate::state::InstanceLink::Unmanaged),
        ..Default::default()
    };
    let job = crate::install::model::InstallJobState::new(
        crate::install::InstallRequest::InstallContentBatch {
            instance_id: "instance".into(),
            items: vec![],
            display_title: "Busy".into(),
            display_icon: None,
        },
    );
    let id = uuid::Uuid::new_v4();
    crate::install::store::insert(
        id,
        &job,
        crate::install::InstallJobStatus::Running,
        &state,
    )
    .await
    .unwrap();
    assert!(
        crate::state::edit_instance("instance", patch.clone(), &state.pool)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM install_jobs WHERE id = ?")
        .bind(id.to_string())
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_unlink BEFORE DELETE ON instance_pack_members BEGIN SELECT RAISE(ABORT, 'test failure'); END;").execute(&state.pool).await.unwrap();
    assert!(
        crate::state::edit_instance("instance", patch, &state.pool)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM resource_pack_unlinks"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    assert!(matches!(
        plans::installed(&state.pool, "instance").await.unwrap()[0].ownership,
        ResourceOwnership::PackManaged { .. }
    ));
    state.pool.close().await;
}

#[tokio::test]
async fn cleanup_sql_failure_recovers_backups_and_external_roots_are_refused() {
    let (temp, state, original, _, root) = fixture().await;
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let plan = local_plan(
        &state,
        original.clone(),
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    execute("instance", &plan, None, &state).await.unwrap();
    let files = super::history_cleanup::scan_with_state("instance", &state)
        .await
        .unwrap();
    let file = &files[0];
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![file.id.clone()],
        &state,
    )
    .await
    .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_cleanup BEFORE UPDATE ON resource_history BEGIN SELECT RAISE(ABORT, 'test failure'); END;").execute(&state.pool).await.unwrap();
    assert!(
        super::history_cleanup::execute_with_state(
            review.id,
            review.revision,
            &state
        )
        .await
        .is_err()
    );
    assert!(
        matches_file(&root.join(&file.relative_path), &file.integrity)
            .await
            .unwrap()
    );
    assert!(
        history::list_with_state("instance", None, &state)
            .await
            .unwrap()[0]
            .backup_available
    );
    let external = temp.path().join("external");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("mod.jar_old.jar.old"), b"external").unwrap();
    sqlx::query(
        "UPDATE instances SET game_dir_override = ? WHERE id = 'instance'",
    )
    .bind(external.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .unwrap();
    assert!(
        super::history_cleanup::scan_with_state("instance", &state)
            .await
            .is_err()
    );
    assert!(external.join("mod.jar_old.jar.old").exists());
    state.pool.close().await;
}

#[tokio::test]
async fn legacy_backup_rollback_clears_stale_release_and_recovers_database_failure()
 {
    let (_temp, state, original, _, root) = fixture().await;
    let backup = "resourcepacks/prefix-old.zip_legacy.zip.old";
    std::fs::write(root.join(backup), b"legacy bytes").unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_legacy_rollback BEFORE DELETE ON instance_content_provider_refs BEGIN SELECT RAISE(ABORT, 'test failure'); END;").execute(&state.pool).await.unwrap();
    assert!(
        crate::state::instances::commands::rollback_project(
            "instance",
            &original.relative_path,
            &state
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(root.join(&original.relative_path)).unwrap(),
        b"old bytes"
    );
    assert_eq!(std::fs::read(root.join(backup)).unwrap(), b"legacy bytes");
    assert_eq!(
        plans::installed(&state.pool, "instance").await.unwrap()[0],
        original
    );
    sqlx::query("DROP TRIGGER fail_legacy_rollback")
        .execute(&state.pool)
        .await
        .unwrap();
    let restored = crate::state::instances::commands::rollback_project(
        "instance",
        &original.relative_path,
        &state,
    )
    .await
    .unwrap();
    assert_eq!(restored, "resourcepacks/legacy.zip.disabled");
    assert_eq!(std::fs::read(root.join(restored)).unwrap(), b"legacy bytes");
    let entry = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(entry.entry_id, original.entry_id);
    assert!(entry.artifact.is_none());
    assert!(entry.known_artifacts.is_empty());
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&state.pool)
            .await
            .unwrap()
            .is_empty()
    );
    state.pool.close().await;
}

#[tokio::test]
async fn rollback_after_unlink_does_not_restore_pack_ownership_or_missing_members()
 {
    let (_temp, state, original, old, root) = fixture().await;
    pack_fixture(&state, &original).await;
    let managed = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    std::fs::write(root.join("new.zip"), b"new bytes").unwrap();
    let plan = local_plan(
        &state,
        managed,
        artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
        "new.zip",
        None,
    )
    .await;
    execute("instance", &plan, None, &state).await.unwrap();
    let history = history::list_with_state("instance", None, &state)
        .await
        .unwrap()
        .remove(0);
    crate::state::edit_instance(
        "instance",
        crate::state::EditInstance {
            link: Some(crate::state::InstanceLink::Unmanaged),
            ..Default::default()
        },
        &state.pool,
    )
    .await
    .unwrap();
    let current = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    let rollback = local_plan(
        &state,
        current,
        old,
        history.backup_relative_path.as_ref().unwrap(),
        Some(history.id),
    )
    .await;
    execute("instance", &rollback, None, &state).await.unwrap();
    let entry = plans::installed(&state.pool, "instance")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(entry.artifact, original.artifact);
    assert_eq!(entry.ownership, ResourceOwnership::UserAdded);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    let mut tx = state.pool.begin().await.unwrap();
    crate::state::instances::adapters::sqlite::instance_rows::upsert_instance_link("instance", &crate::state::InstanceLink::ImportedModpack { project_id: None, version_id: None, name: Some("Local".into()), version_number: None, filename: None }, &mut tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM resource_pack_unlinks"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        0
    );
    pack_fixture(&state, &original).await;
    crate::state::instances::commands::repair_recorded_pack_unlink(
        "instance",
        &state.pool,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM instance_pack_members"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        2
    );
    state.pool.close().await;
}

#[tokio::test]
async fn cleanup_recovery_restores_uncommitted_moves_and_finishes_committed_deletion()
 {
    let (_temp, state, _, _, root) = fixture().await;
    let path = "resourcepacks/current.zip_previous.zip.old";
    std::fs::write(root.join(path), b"old bytes").unwrap();
    let files = super::history_cleanup::scan_with_state("instance", &state)
        .await
        .unwrap();
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![files[0].id.clone()],
        &state,
    )
    .await
    .unwrap();
    let temporary = "resourcepacks/.cleanup-test.old";
    let mut draft = super::history_cleanup::CleanupDraft {
        plan: review.clone(),
        moves: vec![temporary.into()],
        committed: false,
    };
    sqlx::query(
        "UPDATE resource_history_cleanup_plans SET payload = ? WHERE id = ?",
    )
    .bind(serde_json::to_string(&draft).unwrap())
    .bind(review.id.as_str())
    .execute(&state.pool)
    .await
    .unwrap();
    std::fs::rename(root.join(path), root.join(temporary)).unwrap();
    let recovered = super::history_cleanup::scan_with_state("instance", &state)
        .await
        .unwrap();
    assert_eq!(recovered[0].relative_path, path);
    assert!(root.join(path).exists());
    assert!(!root.join(temporary).exists());
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![files[0].id.clone()],
        &state,
    )
    .await
    .unwrap();
    draft.plan = review.clone();
    draft.committed = true;
    sqlx::query("UPDATE resource_history_cleanup_plans SET payload = ?, consumed = 1 WHERE id = ?").bind(serde_json::to_string(&draft).unwrap()).bind(review.id.as_str()).execute(&state.pool).await.unwrap();
    std::fs::rename(root.join(path), root.join(temporary)).unwrap();
    assert!(
        super::history_cleanup::scan_with_state("instance", &state)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!root.join(temporary).exists());
    state.pool.close().await;
}

#[tokio::test]
async fn backup_cleanup_refuses_current_files_and_shared_history_references() {
    let (_temp, state, original, _, root) = fixture().await;
    let path = "resourcepacks/current.zip_previous.zip.old";
    std::fs::write(root.join(path), b"old bytes").unwrap();
    sqlx::query("UPDATE instance_files SET relative_path = ? WHERE id = (SELECT file_id FROM instance_content_entries WHERE id = ?)").bind(path).bind(original.entry_id.as_str()).execute(&state.pool).await.unwrap();
    let files = super::history_cleanup::scan_with_state("instance", &state)
        .await
        .unwrap();
    assert!(
        super::history_cleanup::prepare_with_state(
            "instance",
            vec![files[0].id.clone()],
            &state
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE instance_files SET relative_path = ? WHERE id = (SELECT file_id FROM instance_content_entries WHERE id = ?)").bind(&original.relative_path).bind(original.entry_id.as_str()).execute(&state.pool).await.unwrap();
    let history = history::history_entry(
        "instance",
        &original,
        original.clone(),
        &local_plan(
            &state,
            original.clone(),
            artifact(SourceId::curseforge(), "123", "456", b"new bytes"),
            path,
            None,
        )
        .await,
        Some(path.into()),
    );
    let mut second = history.clone();
    second.id = HistoryId::new(uuid::Uuid::new_v4().to_string()).unwrap();
    for entry in [&history, &second] {
        sqlx::query("INSERT INTO resource_history(id, instance_id, entry_id, recorded_at, payload, backup_relative_path, backup_available) VALUES (?, 'instance', ?, 1, ?, ?, 1)").bind(entry.id.as_str()).bind(original.entry_id.as_str()).bind(serde_json::to_string(entry).unwrap()).bind(path).execute(&state.pool).await.unwrap();
    }
    assert!(
        super::history_cleanup::prepare_with_state(
            "instance",
            vec![history.id.clone()],
            &state
        )
        .await
        .is_err()
    );
    let review = super::history_cleanup::prepare_with_state(
        "instance",
        vec![history.id, second.id],
        &state,
    )
    .await
    .unwrap();
    assert_eq!(review.files.len(), 1);
    assert_eq!(review.entries.len(), 2);
    let result =
        super::history_cleanup::execute_with_state(review.id, 1, &state)
            .await
            .unwrap();
    assert_eq!(result.removed.len(), 2);
    assert!(!root.join(path).exists());
    state.pool.close().await;
}
