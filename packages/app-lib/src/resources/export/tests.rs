use super::{legacy::*, refresh::*};
use crate::resources::{cache::ResourceCache, providers::*, *};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

struct Provider {
    source: SourceId,
    calls: Mutex<Vec<ProviderQuery>>,
    failed: AtomicBool,
    matched: bool,
}

#[tokio::test]
async fn export_reference_reads_are_local_and_do_not_create_business_identity()
{
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    assert!(known_references(&pool, "unknown").await.unwrap().is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
}

#[tokio::test]
async fn session_addition_cache_clear_and_archive_signature_do_not_expand_the_file_set()
 {
    let (root, service, mr, _) = setup(false).await;
    std::fs::write(root.path().join("a.jar"), "one").unwrap();
    std::fs::write(root.path().join("b.bin"), b"PK\x03\x04test").unwrap();
    let mut draft = draft(vec![file("a.jar"), file("b.bin")]);
    run(&service, &mut draft, root.path(), vec!["a.jar".into()], 1).await;
    let before = mr.calls.lock().unwrap().len();
    service.cache().clear().await.unwrap();
    let result = run(
        &service,
        &mut draft,
        root.path(),
        vec!["a.jar".into(), "b.bin".into()],
        2,
    )
    .await;
    assert_eq!(before + 1, mr.calls.lock().unwrap().len());
    assert_eq!(result.files.len(), 2);
    assert!(result.files[1].snapshot.identifiable);
    std::fs::write(root.path().join("new.jar"), "new").unwrap();
    let result =
        run(&service, &mut draft, root.path(), vec!["a.jar".into()], 3).await;
    assert_eq!(result.files.len(), 1);
    assert_eq!(before + 1, mr.calls.lock().unwrap().len());
    let bad = refresh_local(
        &service,
        &mut draft,
        root.path(),
        vec!["new.jar".into()],
        4,
        RequestId::new("new").unwrap(),
        HashMap::new(),
        &CancellationToken::new(),
        &|_| {},
    )
    .await;
    assert!(matches!(bad, Err(ResourceError::InvalidRequest(_))));
}
#[async_trait]
impl ResourceProvider for Provider {
    fn source(&self) -> ResourceSource {
        ResourceSource {
            id: self.source.clone(),
            available: true,
            types: vec![ResourceType::Mod],
            capabilities: vec![
                SourceCapability::IdentifyHash,
                SourceCapability::Project,
                SourceCapability::Releases,
            ],
            supported_search_sorts: vec![],
        }
    }
    fn namespace(&self) -> String {
        "export-test".into()
    }
    fn batch_limit(&self, _: &ProviderQuery) -> usize {
        50
    }
    async fn fetch_many(
        &self,
        queries: &[ProviderQuery],
    ) -> Vec<ResourceResult<ProviderValue>> {
        self.calls.lock().unwrap().extend_from_slice(queries);
        queries.iter().map(|query| {
            if self.failed.load(Ordering::SeqCst) { return Err(ResourceError::Network { status: Some(503) }); }
            match query {
                ProviderQuery::IdentifyExport { integrity, .. } => {
                    if !self.matched { return Ok(ProviderValue::Identified(vec![])); }
                    let reference = SourceReleaseRef { project: SourceProjectRef { source: self.source.clone(), project_id: SourceProjectId::new("12345678").unwrap() }, release_id: SourceReleaseId::new("87654321").unwrap() };
                    let artifact = ResourceArtifact { reference: SourceArtifactRef { release: reference.clone(), artifact_id: SourceArtifactId::new("file.jar").unwrap() }, file_name: Some("file.jar".into()), role: ArtifactRole::Primary, available: Some(true), integrity: integrity.clone(), acquisition: ArtifactAcquisition::PublicDownload { urls: vec!["https://cdn.modrinth.com/file.jar".into()] }, dependencies: vec![] };
                    Ok(ProviderValue::Identified(vec![ReleaseDocument { release: ResourceRelease { reference, version: "1".into(), channel: ReleaseChannel::Release, published_at: None, compatibility: ReleaseCompatibility { game_versions: vec![], loaders: vec![] }, artifacts: vec![artifact], equivalent_releases: vec![] }, changelog: None, protocol: None }]))
                },
                ProviderQuery::Project { id, .. } => Ok(ProviderValue::Project(ProjectDocument { metadata: SourceProjectMetadata { project: SourceProjectRef { source: self.source.clone(), project_id: id.clone() }, slug: None, title: "Test".into(), summary: String::new(), icon_url: None, types: vec![ResourceType::Mod], authors: vec![], license: None, downloads: None, followers: None, created_at: None, updated_at: None, first_published_at: None, latest_published_at: None, categories: vec![], game_versions: vec![], loaders: vec![], gallery: vec![], environments: vec![] }, description: ResourceText { source: self.source.clone(), format: TextFormat::Plain, body: String::new() }, protocol: None })),
                _ => panic!("export must not traverse releases or download: {query:?}"),
            }
        }).collect()
    }
}
fn file(path: &str) -> ExportFileSnapshot {
    ExportFileSnapshot {
        relative_path: path.into(),
        integrity: ArtifactIntegrity {
            size: Some(4),
            hashes: vec![],
        },
        revision: 1,
        identifiable: path.ends_with(".jar"),
        modified_at: None,
        category_id: "mods".into(),
        category_selected: true,
    }
}
fn draft(files: Vec<ExportFileSnapshot>) -> ExportDraft {
    ExportDraft::new(ResourceExportScan {
        session_id: SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        instance_id: "instance".into(),
        instance_revision: 1,
        files,
        categories: vec![],
        excluded_paths: vec![],
        directories: vec![],
        requirements: ExportGameRequirements {
            game_version: "1.21.1".into(),
            loader: None,
            loader_version: None,
        },
    })
}
fn info() -> ExportPackageInfo {
    ExportPackageInfo {
        name: "My package".into(),
        version: "2".into(),
        summary: "Summary".into(),
        author: None,
    }
}
async fn setup(
    matched: bool,
) -> (
    tempfile::TempDir,
    Arc<ResourceService>,
    Arc<Provider>,
    Arc<Provider>,
) {
    let temp = tempfile::tempdir().unwrap();
    let cache = Arc::new(
        ResourceCache::open(temp.path().join("cache/resource-cache.db")).await,
    );
    let mr = Arc::new(Provider {
        source: SourceId::modrinth(),
        calls: Mutex::new(vec![]),
        failed: AtomicBool::new(false),
        matched,
    });
    let cf = Arc::new(Provider {
        source: SourceId::curseforge(),
        calls: Mutex::new(vec![]),
        failed: AtomicBool::new(false),
        matched: false,
    });
    let service =
        ResourceService::new(cache, vec![mr.clone(), cf.clone()]).unwrap();
    (temp, service, mr, cf)
}
async fn run(
    service: &Arc<ResourceService>,
    draft: &mut ExportDraft,
    root: &std::path::Path,
    selected: Vec<String>,
    revision: u32,
) -> ExportResolutionSession {
    refresh_local(
        service,
        draft,
        root,
        selected,
        revision,
        RequestId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
        HashMap::new(),
        &CancellationToken::new(),
        &|_| {},
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn force_refresh_all_selected_unknown_files_bypasses_warm_negative_cache_and_reuses_session_evidence()
 {
    let (root, service, mr, cf) = setup(true).await;
    let mut files = vec![];
    for index in 0..105 {
        let path = format!("file-{index}.jar");
        std::fs::write(root.path().join(&path), format!("{index:04}")).unwrap();
        files.push(file(&path));
    }
    std::fs::write(root.path().join("unselected.jar"), "data").unwrap();
    files.push(file("unselected.jar"));
    std::fs::write(root.path().join("options.txt"), "data").unwrap();
    files.push(file("options.txt"));
    let selected = files
        .iter()
        .filter(|f| f.relative_path != "unselected.jar")
        .map(|f| f.relative_path.clone())
        .collect::<Vec<_>>();
    let mut draft = draft(files);
    let (frozen, fingerprint) =
        freeze(root.path(), &draft.scan.files[0], &CancellationToken::new())
            .await
            .unwrap();
    mr.failed.store(false, Ordering::SeqCst);
    service
        .query(
            &SourceId::modrinth(),
            ProviderQuery::IdentifyExport {
                integrity: frozen.integrity,
                fingerprint: Some(fingerprint),
            },
            CachePolicy::UseFresh,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    mr.calls.lock().unwrap().clear();
    let (frozen, fingerprint) =
        freeze(root.path(), &draft.scan.files[0], &CancellationToken::new())
            .await
            .unwrap();
    service
        .query(
            &SourceId::curseforge(),
            ProviderQuery::IdentifyExport {
                integrity: frozen.integrity,
                fingerprint: Some(fingerprint),
            },
            CachePolicy::UseFresh,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    cf.calls.lock().unwrap().clear();
    let result =
        run(&service, &mut draft, root.path(), selected.clone(), 1).await;
    assert_eq!(result.state, ExportResolutionState::Ready);
    assert_eq!(
        mr.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|q| matches!(q, ProviderQuery::IdentifyExport { .. }))
            .count(),
        105
    );
    assert_eq!(cf.calls.lock().unwrap().len(), 105);
    assert!(!mr.calls.lock().unwrap().iter().any(|q| matches!(
        q,
        ProviderQuery::Release { .. } | ProviderQuery::Releases { .. }
    )));
    let before = mr.calls.lock().unwrap().len();
    run(&service, &mut draft, root.path(), selected.clone(), 2).await;
    assert_eq!(before, mr.calls.lock().unwrap().len());
    std::fs::write(root.path().join(&selected[0]), "edit").unwrap();
    run(&service, &mut draft, root.path(), selected.clone(), 3).await;
    assert_eq!(before + 1, mr.calls.lock().unwrap().len());
    let pack = mr_pack(&draft, info(), &[]).unwrap();
    assert_eq!(pack.files.len(), 105);
    assert_eq!(pack.name, "My package");
    assert!(
        pack.files
            .iter()
            .all(|file| file.env.is_none() && file.hashes.len() == 2)
    );
    let mut new = ExportDraft::new(draft.scan.clone());
    run(&service, &mut new, root.path(), selected, 1).await;
    assert_eq!(before + 107, mr.calls.lock().unwrap().len());
}

#[tokio::test]
async fn source_failure_is_not_cached_success_and_embedding_requires_exact_snapshot()
 {
    let (root, service, mr, cf) = setup(false).await;
    std::fs::write(root.path().join("file.jar"), "data").unwrap();
    let mut draft = draft(vec![file("file.jar")]);
    let first = run(
        &service,
        &mut draft,
        root.path(),
        vec!["file.jar".into()],
        1,
    )
    .await;
    assert_eq!(first.state, ExportResolutionState::Ready);
    let mut reopened = ExportDraft::new(draft.scan.clone());
    cf.failed.store(true, Ordering::SeqCst);
    let result = run(
        &service,
        &mut reopened,
        root.path(),
        vec!["file.jar".into()],
        1,
    )
    .await;
    assert_eq!(result.state, ExportResolutionState::NeedsDecision);
    assert!(validate_choices(&reopened, 1, &[]).is_err());
    let choice = ExportEmbeddedFailureChoice {
        snapshot: result.files[0].snapshot.clone(),
        source: SourceId::curseforge(),
    };
    validate_choices(&reopened, 1, &[choice.clone()]).unwrap();
    let mr_before = mr.calls.lock().unwrap().len();
    cf.failed.store(false, Ordering::SeqCst);
    run(
        &service,
        &mut reopened,
        root.path(),
        vec!["file.jar".into()],
        2,
    )
    .await;
    assert_eq!(mr_before, mr.calls.lock().unwrap().len());
    assert!(validate_choices(&reopened, 2, &[choice]).is_err());
    std::fs::remove_file(root.path().join("file.jar")).unwrap();
    let result = run(
        &service,
        &mut reopened,
        root.path(),
        vec!["file.jar".into()],
        3,
    )
    .await;
    assert!(result.files[0].local_error.is_some());
    assert!(validate_choices(&reopened, 3, &[]).is_err());
}

#[tokio::test]
async fn fingerprint_and_hashes_use_current_bytes_and_cancel_before_network() {
    let (root, service, mr, _) = setup(false).await;
    let bytes = b"file \n\r\tbytes";
    std::fs::write(root.path().join("file.jar"), bytes).unwrap();
    let (_, fingerprint) =
        freeze(root.path(), &file("file.jar"), &CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(
        fingerprint,
        crate::api::curseforge::compute_fingerprint(bytes)
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut draft = draft(vec![file("file.jar")]);
    let result = refresh_local(
        &service,
        &mut draft,
        root.path(),
        vec!["file.jar".into()],
        1,
        RequestId::new("cancel").unwrap(),
        HashMap::new(),
        &cancel,
        &|_| {},
    )
    .await;
    assert!(matches!(result, Err(ResourceError::Canceled)));
    assert!(mr.calls.lock().unwrap().is_empty());
}
