use super::*;
use crate::resources::plans::OperationDraft;
use std::collections::HashMap;

#[tokio::test]
async fn the_same_datapack_in_two_worlds_produces_two_scoped_actions() {
    let mut old = release("pack0000", "1", 'a');
    let mut new = release("pack0000", "2", 'b');
    for release in [&mut old, &mut new] {
        release.artifacts[0].file_name =
            Some(format!("{}.zip", release.version));
    }
    let (_directory, service) = service(vec![old.clone(), new.clone()]).await;
    let mut roots = Vec::new();
    let mut entries = Vec::new();
    for world in ["one", "two"] {
        let scope = InstallScope::World {
            relative_path: format!("saves/{world}"),
        };
        let mut root = selection(world, &new, Some(world));
        root.scope = scope.clone();
        root.resource_type = ResourceType::DataPack;
        let mut entry = installed(
            &old,
            world,
            &format!("saves/{world}/datapacks/custom.zip"),
        );
        entry.scope = scope;
        entry.resource_type = ResourceType::DataPack;
        roots.push(root);
        entries.push(entry);
    }
    let plan = service
        .build_operation(
            draft(roots, ResourceOperationIntent::Update),
            &target(),
            &entries,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
    assert_eq!(plan.actions.len(), 2);
    assert_eq!(
        plan.actions[0].target_relative_path,
        "saves/one/datapacks/custom.zip"
    );
    assert_eq!(
        plan.actions[1].target_relative_path,
        "saves/two/datapacks/custom.zip"
    );
    service.cache().close().await;
}

#[tokio::test]
async fn confirmed_cross_source_dependency_preserves_both_parent_requirements()
{
    let mut a = release("root0000", "1", 'a');
    let mut mr_dep = release("dep00000", "1", 'c');
    let mut cf_dep = release("123", "1", 'c');
    let mut cf_root = release("456", "1", 'd');
    for release in [&mut cf_dep, &mut cf_root] {
        release.reference.project.source = SourceId::curseforge();
        release.reference.release_id = SourceReleaseId::new(
            if release.reference.project.project_id.as_str() == "123" {
                "101"
            } else {
                "102"
            },
        )
        .unwrap();
        release.artifacts[0].reference.release = release.reference.clone();
    }
    mr_dep.reference.release_id = SourceReleaseId::new("depver01").unwrap();
    mr_dep.artifacts[0].reference.release = mr_dep.reference.clone();
    requires(&mut a, &mr_dep, ResourceDependencyKind::Required);
    requires(&mut cf_root, &cf_dep, ResourceDependencyKind::Required);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    let store = crate::state::resources::ResourceStore::new(pool.clone());
    let ProviderValue::Project(left) =
        project_value(SourceId::modrinth(), "dep00000", "Dependency")
    else {
        panic!()
    };
    let ProviderValue::Project(right) =
        project_value(SourceId::curseforge(), "123", "Dependency")
    else {
        panic!()
    };
    store
        .confirm_projects(
            &left.metadata,
            &right.metadata,
            &mr_dep.artifacts[0],
            &cf_dep.artifacts[0],
        )
        .await
        .unwrap();
    let (directory, cache) = cache().await;
    let service = ResourceService::with_store(
        cache.clone(),
        vec![
            Arc::new(PlanProvider {
                source: SourceId::modrinth(),
                releases: vec![a.clone(), mr_dep],
                requests: Mutex::new(vec![]),
            }),
            Arc::new(PlanProvider {
                source: SourceId::curseforge(),
                releases: vec![cf_root.clone(), cf_dep],
                requests: Mutex::new(vec![]),
            }),
        ],
        store,
    )
    .unwrap();
    let plan = service
        .build_operation(
            draft(
                vec![
                    selection("mr", &a, None),
                    selection("cf", &cf_root, None),
                ],
                ResourceOperationIntent::Install,
            ),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
    assert_eq!(plan.actions.len(), 3);
    assert_eq!(plan.edges.len(), 2);
    assert_eq!(plan.edges[0].child, plan.edges[1].child);
    cache.close().await;
    pool.close().await;
    drop(directory);
}

struct PlanProvider {
    source: SourceId,
    releases: Vec<ResourceRelease>,
    requests: Mutex<Vec<ProviderQuery>>,
}
#[async_trait]
impl ResourceProvider for PlanProvider {
    fn source(&self) -> ResourceSource {
        ResourceSource {
            id: self.source.clone(),
            types: vec![ResourceType::Mod, ResourceType::DataPack],
            capabilities: vec![
                SourceCapability::Project,
                SourceCapability::Releases,
            ],
            available: true,
            supported_search_sorts: vec![],
        }
    }
    fn namespace(&self) -> String {
        "plan-fixture".into()
    }
    fn batch_limit(&self, _: &ProviderQuery) -> usize {
        100
    }
    async fn fetch_many(
        &self,
        queries: &[ProviderQuery],
    ) -> Vec<ResourceResult<ProviderValue>> {
        self.requests.lock().unwrap().extend_from_slice(queries);
        queries
            .iter()
            .map(|query| match query {
                ProviderQuery::Project { id, .. } => {
                    let mut value = project_value(
                        self.source.clone(),
                        id.as_str(),
                        id.as_str(),
                    );
                    if let ProviderValue::Project(document) = &mut value {
                        document.metadata.types.push(ResourceType::DataPack);
                    }
                    Ok(value)
                }
                ProviderQuery::Releases { project_id, .. } => {
                    Ok(ProviderValue::Releases(SourcePage {
                        entries: self
                            .releases
                            .iter()
                            .filter(|release| {
                                &release.reference.project.project_id
                                    == project_id
                            })
                            .cloned()
                            .collect(),
                        cursor: None,
                        total: ipc::ResourceTotal {
                            value: None,
                            precision: ipc::TotalPrecision::Unknown,
                        },
                        window_exhausted: false,
                    }))
                }
                ProviderQuery::Release { release_id, .. } => self
                    .releases
                    .iter()
                    .find(|release| &release.reference.release_id == release_id)
                    .cloned()
                    .map(|release| {
                        ProviderValue::Release(ReleaseDocument {
                            release,
                            changelog: None,
                            protocol: None,
                        })
                    })
                    .ok_or(ResourceError::Unavailable),
                _ => Err(ResourceError::Unsupported("fixture query".into())),
            })
            .collect()
    }
}
fn release(project: &str, version: &str, byte: char) -> ResourceRelease {
    let reference = SourceReleaseRef {
        project: SourceProjectRef {
            source: SourceId::modrinth(),
            project_id: SourceProjectId::new(project).unwrap(),
        },
        release_id: SourceReleaseId::new(format!("{project}-{version}"))
            .unwrap(),
    };
    ResourceRelease {
        reference: reference.clone(),
        version: version.into(),
        channel: ReleaseChannel::Release,
        published_at: Some(
            chrono::DateTime::from_timestamp(
                version.parse::<i64>().unwrap() * 1000,
                0,
            )
            .unwrap(),
        ),
        compatibility: ReleaseCompatibility {
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
        },
        artifacts: vec![ResourceArtifact {
            reference: SourceArtifactRef {
                release: reference,
                artifact_id: SourceArtifactId::new(format!(
                    "{project}-{version}.jar"
                ))
                .unwrap(),
            },
            file_name: Some(format!("{project}-{version}.jar")),
            role: ArtifactRole::Primary,
            available: Some(true),
            integrity: ArtifactIntegrity {
                size: Some(100),
                hashes: vec![ContentHash {
                    algorithm: HashAlgorithm::Sha1,
                    value: byte.to_string().repeat(40),
                }],
            },
            acquisition: ArtifactAcquisition::PublicDownload {
                urls: vec![format!(
                    "https://example.com/{project}-{version}.jar"
                )],
            },
            dependencies: vec![],
        }],
        equivalent_releases: vec![],
    }
}
fn requires(
    parent: &mut ResourceRelease,
    child: &ResourceRelease,
    kind: ResourceDependencyKind,
) {
    parent.artifacts[0].dependencies.push(ArtifactDependency {
        target: DependencyTarget::Release {
            reference: child.reference.clone(),
        },
        kind,
        version_constraint: None,
    });
}
fn selection(
    id: &str,
    release: &ResourceRelease,
    entry: Option<&str>,
) -> ResourceSelection {
    ResourceSelection {
        id: SelectionId::new(id).unwrap(),
        resource: ResourceRef::Source {
            project: release.reference.project.clone(),
        },
        release: ReleaseSelection::Artifact {
            reference: release.artifacts[0].reference.clone(),
        },
        resource_type: ResourceType::Mod,
        scope: InstallScope::Instance,
        entry_id: entry.map(|id| InstalledEntryId::new(id).unwrap()),
    }
}
fn installed(
    release: &ResourceRelease,
    id: &str,
    path: &str,
) -> InstalledResource {
    InstalledResource {
        entry_id: InstalledEntryId::new(id).unwrap(),
        instance_id: "instance".into(),
        resource_id: None,
        artifact: Some(release.artifacts[0].reference.clone()),
        known_artifacts: vec![release.artifacts[0].reference.clone()],
        integrity: release.artifacts[0].integrity.clone(),
        relative_path: path.into(),
        original_file_name: release.artifacts[0].file_name.clone(),
        resource_type: ResourceType::Mod,
        scope: InstallScope::Instance,
        enabled: !path.ends_with(".disabled"),
        missing: false,
        ownership: ResourceOwnership::UserAdded,
    }
}
fn draft(
    selections: Vec<ResourceSelection>,
    intent: ResourceOperationIntent,
) -> OperationDraft {
    OperationDraft {
        plan: ResourceOperationPlan {
            schema_version: ResourceContractVersion,
            id: PlanId::new("plan").unwrap(),
            revision: 1,
            instance_id: "instance".into(),
            instance_revision: u32::MAX as u64 + 1,
            intent,
            active_roots: selections
                .iter()
                .map(|selection| selection.id.clone())
                .collect(),
            selections,
            nodes: vec![],
            edges: vec![],
            actions: vec![],
            issues: vec![],
            exceptions: vec![],
            selection_reasons: vec![],
        },
        candidates: vec![],
        install_dependencies: true,
        local_files: HashMap::new(),
        restore_history: None,
    }
}
fn target() -> ipc::ResourceTarget {
    ipc::ResourceTarget {
        game_versions: vec!["1.20.1".into()],
        loaders: vec!["fabric".into()],
        channels: vec![ReleaseChannel::Release],
    }
}
async fn service(
    releases: Vec<ResourceRelease>,
) -> (tempfile::TempDir, Arc<ResourceService>) {
    let (directory, cache) = cache().await;
    let provider = Arc::new(PlanProvider {
        source: SourceId::modrinth(),
        releases,
        requests: Mutex::new(vec![]),
    });
    (
        directory,
        ResourceService::new(cache, vec![provider]).unwrap(),
    )
}

#[tokio::test]
async fn duplicate_updates_preserve_each_entry_and_merge_shared_dependency() {
    let old = release("root0000", "1", 'a');
    let mut new = release("root0000", "2", 'b');
    let dependency = release("dep00000", "1", 'c');
    requires(&mut new, &dependency, ResourceDependencyKind::Required);
    let (_directory, service) =
        service(vec![old.clone(), new.clone(), dependency.clone()]).await;
    let selections = vec![
        selection("first", &new, Some("a")),
        selection("second", &new, Some("b")),
    ];
    let current = vec![
        installed(&old, "a", "mods/custom-prefix.jar.disabled"),
        installed(&old, "b", "mods/copy.jar"),
    ];
    let plan = service
        .build_operation(
            draft(selections, ResourceOperationIntent::Update),
            &target(),
            &current,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
    assert_eq!(plan.actions.len(), 3);
    assert_eq!(plan.actions[0].entry_id.as_str(), "a");
    assert_eq!(
        plan.actions[0].target_relative_path,
        "mods/custom-prefix.jar.disabled"
    );
    assert_eq!(plan.actions[1].entry_id.as_str(), "b");
    assert_eq!(plan.actions[1].target_relative_path, "mods/copy.jar");
    assert_eq!(
        plan.nodes
            .iter()
            .filter(|node| node.artifact.as_ref().is_some_and(|artifact| {
                artifact.reference.release.project.project_id.as_str()
                    == "dep00000"
            }))
            .count(),
        1
    );
    assert_eq!(plan.edges.len(), 2);
    service.cache().close().await;
}

#[tokio::test]
async fn optional_dependency_closure_only_appears_after_activation_and_can_be_removed()
 {
    let mut a = release("root0000", "1", 'a');
    let mut b = release("opt00000", "1", 'b');
    let c = release("dep00000", "1", 'c');
    requires(&mut b, &c, ResourceDependencyKind::Required);
    requires(&mut a, &b, ResourceDependencyKind::Optional);
    let (_directory, service) = service(vec![a.clone(), b, c]).await;
    let first = service
        .build_operation(
            draft(
                vec![selection("root", &a, None)],
                ResourceOperationIntent::Install,
            ),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(first.plan.actions.len(), 1);
    assert_eq!(first.candidates.len(), 1);
    assert_eq!(first.plan.nodes.len(), 2);
    let mut active = first.clone();
    active.plan.selections.push(first.candidates[0].clone());
    active
        .plan
        .active_roots
        .push(first.candidates[0].id.clone());
    let active = service
        .build_operation(active, &target(), &[], &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(active.plan.actions.len(), 3);
    assert!(active.plan.issues.is_empty());
    let mut removed = active;
    removed.plan.selections.truncate(1);
    removed.plan.active_roots.truncate(1);
    let removed = service
        .build_operation(removed, &target(), &[], &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(removed.plan.actions.len(), 1);
    assert_eq!(removed.plan.nodes.len(), 2);
    service.cache().close().await;
}

#[tokio::test]
async fn duplicate_with_target_bytes_does_not_satisfy_the_entry_being_updated()
{
    let old = release("root0000", "1", 'a');
    let new = release("root0000", "2", 'b');
    let (_directory, service) = service(vec![old.clone(), new.clone()]).await;
    let mut already_new = installed(&new, "copy", "mods/copy.jar");
    already_new.enabled = true;
    let plan = service
        .build_operation(
            draft(
                vec![selection("root", &new, Some("old"))],
                ResourceOperationIntent::Update,
            ),
            &target(),
            &[installed(&old, "old", "mods/original.jar"), already_new],
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].entry_id.as_str(), "old");
    assert_eq!(plan.nodes[0].state, DependencyNodeState::Update);
    assert!(plan.nodes[0].installed_entries.is_empty());
    service.cache().close().await;
}

#[tokio::test]
async fn optional_dependency_of_a_required_dependency_stays_a_candidate() {
    let mut a = release("root0000", "1", 'a');
    let mut c = release("dep00000", "1", 'c');
    let e = release("opt00000", "1", 'e');
    requires(&mut a, &c, ResourceDependencyKind::Required);
    requires(&mut c, &e, ResourceDependencyKind::Optional);
    let (_directory, service) =
        service(vec![a.clone(), c.clone(), e.clone()]).await;
    let draft = service
        .build_operation(
            draft(
                vec![selection("a", &a, None)],
                ResourceOperationIntent::Install,
            ),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(draft.plan.issues.is_empty(), "{:?}", draft.plan.issues);
    assert_eq!(draft.plan.actions.len(), 2);
    assert_eq!(draft.candidates.len(), 1);
    assert!(matches!(
        &draft.candidates[0].resource,
        ResourceRef::Source { project }
            if project.project_id.as_str() == "opt00000"
    ));
    let candidate = &draft.candidates[0];
    assert!(draft.plan.nodes.iter().any(|node| {
        node.id == candidate.id
            && node.state == DependencyNodeState::OptionalCandidate
    }));
    let parent = draft.plan.nodes.iter().find(|node| {
        node.artifact.as_ref().is_some_and(|artifact| {
            artifact.reference.release.project.project_id.as_str() == "dep00000"
        })
    });
    let parent = parent.expect("required dependency node");
    assert!(draft.plan.edges.iter().any(|edge| {
        edge.parent == parent.id
            && edge.child == candidate.id
            && edge.kind == ResourceDependencyKind::Optional
    }));
    service.cache().close().await;
}

#[tokio::test]
async fn curseforge_automatic_roots_and_optional_candidates_resolve_under_channel_targets()
 {
    fn file_for(id: u64) -> Value {
        let mut file: Value = serde_json::from_str(fixtures::CF_IRIS).unwrap();
        if id == 5425379 {
            file["id"] = json!(5425379);
            file["modId"] = json!(394468);
            file["fileName"] = json!("optional-dependency-1.0.jar");
            file["displayName"] = json!("Optional Dependency 1.0");
            file["dependencies"] = json!([]);
        } else {
            file["dependencies"] =
                json!([{"modId": 394468, "relationType": 2}]);
        }
        file
    }
    let transport = FixtureTransport::new(|_, request| {
        match request.path.as_str() {
            "/v1/mods/455508/files" | "/v1/mods/394468/files" => {
                assert!(
                    request
                        .query
                        .contains(&("gameVersion".into(), "1.20.1".into()))
                );
                assert!(
                    request
                        .query
                        .contains(&("modLoaderType".into(), "4".into()))
                );
                let id = if request.path.starts_with("/v1/mods/394468") {
                    5425379
                } else {
                    5425378
                };
                Ok(json!({
                    "data": [file_for(id)],
                    "pagination": {"index": 0, "resultCount": 1, "totalCount": 1}
                }))
            }
            "/v1/mods/files" => {
                let ids = request.body.as_ref().unwrap()["fileIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect::<Vec<_>>();
                Ok(json!({
                    "data": ids.into_iter().map(file_for).collect::<Vec<_>>()
                }))
            }
            "/v1/mods" => {
                let ids = request.body.as_ref().unwrap()["modIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect::<Vec<_>>();
                Ok(json!({"data": ids.into_iter().map(|id| json!({
                    "id": id,
                    "name": format!("Project {id}"),
                    "slug": format!("project-{id}"),
                    "classId": 6
                })).collect::<Vec<_>>()}))
            }
            path => panic!("unexpected endpoint {path}"),
        }
    });
    let (_directory, cache) = cache().await;
    let service = ResourceService::new(
        cache.clone(),
        vec![Arc::new(Available(Arc::new(CurseForgeProvider::new(
            transport.clone(),
        ))))],
    )
    .unwrap();
    let root = ResourceSelection {
        id: SelectionId::new("root").unwrap(),
        resource: ResourceRef::Source {
            project: SourceProjectRef {
                source: SourceId::curseforge(),
                project_id: SourceProjectId::new("455508").unwrap(),
            },
        },
        release: ReleaseSelection::Automatic,
        resource_type: ResourceType::Mod,
        scope: InstallScope::Instance,
        entry_id: None,
    };
    let first = service
        .build_operation(
            draft(vec![root], ResourceOperationIntent::Install),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(first.plan.issues.is_empty(), "{:?}", first.plan.issues);
    assert_eq!(first.plan.actions.len(), 1);
    assert_eq!(first.candidates.len(), 1);
    assert!(first.plan.nodes.iter().any(|node| {
        node.state == DependencyNodeState::OptionalCandidate
            && matches!(&node.resource, ResourceRef::Source { project } if project.project_id.as_str() == "394468")
    }));
    let mut active = first;
    let candidate = active.candidates[0].clone();
    active.plan.selections.push(candidate.clone());
    active.plan.active_roots.push(candidate.id.clone());
    let active = service
        .build_operation(active, &target(), &[], &CancellationToken::new())
        .await
        .unwrap();
    assert!(active.plan.issues.is_empty(), "{:?}", active.plan.issues);
    assert_eq!(active.plan.actions.len(), 2);
    assert_eq!(active.plan.edges.len(), 1);
    assert!(active.plan.nodes.iter().any(|node| {
        node.artifact.as_ref().is_some_and(|artifact| {
            artifact.reference.release.project.project_id.as_str() == "394468"
        })
    }));
    service.cache().close().await;
}

#[tokio::test]
async fn dependency_conflicts_and_foreign_entry_ownership_block_a_plan() {
    let mut a = release("root0000", "1", 'a');
    let mut d = release("root1111", "1", 'd');
    let c1 = release("dep00000", "1", 'c');
    let c2 = release("dep00000", "2", 'e');
    requires(&mut a, &c1, ResourceDependencyKind::Required);
    requires(&mut d, &c2, ResourceDependencyKind::Required);
    let (_directory, service) =
        service(vec![a.clone(), d.clone(), c1, c2]).await;
    let plan = service
        .build_operation(
            draft(
                vec![selection("a", &a, None), selection("d", &d, None)],
                ResourceOperationIntent::Install,
            ),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == ResourceIssueCode::VersionConflict
                && issue.blocking)
    );
    let current = vec![installed(&d, "entry", "mods/entry.jar")];
    let plan = service
        .build_operation(
            draft(
                vec![selection("foreign", &a, Some("entry"))],
                ResourceOperationIntent::SwitchVersion,
            ),
            &target(),
            &current,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == ResourceIssueCode::ArtifactMismatch
                && issue.blocking)
    );
    service.cache().close().await;
}

#[tokio::test]
async fn an_existing_root_does_not_swallow_a_different_exact_dependency_artifact()
 {
    let mut a = release("root0000", "1", 'a');
    let d1 = release("dep00000", "1", 'c');
    let d2 = release("dep00000", "2", 'e');
    requires(&mut a, &d2, ResourceDependencyKind::Required);
    let (_directory, service) = service(vec![a.clone(), d1.clone(), d2]).await;
    let plan = service
        .build_operation(
            draft(
                vec![
                    selection("dependency-root", &d1, None),
                    selection("parent", &a, None),
                ],
                ResourceOperationIntent::Install,
            ),
            &target(),
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == ResourceIssueCode::VersionConflict
                && issue.blocking)
    );
    service.cache().close().await;
}

#[tokio::test]
async fn disabled_unchanged_file_does_not_trigger_reinstall_and_unknown_requires_explicit_exception()
 {
    let release = release("root0000", "1", 'a');
    let (_directory, service) = service(vec![release.clone()]).await;
    let current =
        vec![installed(&release, "entry", "mods/custom.jar.disabled")];
    let plan = service
        .build_operation(
            draft(
                vec![selection("root", &release, Some("entry"))],
                ResourceOperationIntent::Update,
            ),
            &target(),
            &current,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.actions.is_empty());
    let mut environment = target();
    environment.game_versions.clear();
    let plan = service
        .build_operation(
            draft(
                vec![selection("root", &release, None)],
                ResourceOperationIntent::Install,
            ),
            &environment,
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .plan;
    assert!(plan.issues.iter().any(|issue| issue.code
        == ResourceIssueCode::CompatibilityUnknown
        && issue.blocking));
    service.cache().close().await;
}
