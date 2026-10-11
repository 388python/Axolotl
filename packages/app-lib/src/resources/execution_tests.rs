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
    let target = artifact(SourceId::curseforge(), "123", "456", b"new bytes");
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
