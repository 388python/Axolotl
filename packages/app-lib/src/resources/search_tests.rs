use super::*;
use crate::resources::ipc::*;

struct SearchProvider {
    source: SourceId,
    entries: Vec<SourceProjectMetadata>,
    calls: Mutex<Vec<ProviderQuery>>,
    fail: AtomicBool,
}

fn metadata(
    source: SourceId,
    index: usize,
    title: &str,
) -> SourceProjectMetadata {
    let id = if source == SourceId::curseforge() {
        (index + 1).to_string()
    } else {
        format!("{index:08}")
    };
    let ProviderValue::Project(document) = project_value(source, &id, title)
    else {
        unreachable!()
    };
    document.metadata
}

impl SearchProvider {
    fn new(source: SourceId, titles: &[String]) -> Arc<Self> {
        Arc::new(Self {
            entries: titles
                .iter()
                .enumerate()
                .map(|(index, title)| metadata(source.clone(), index, title))
                .collect(),
            source,
            calls: Mutex::new(vec![]),
            fail: AtomicBool::new(false),
        })
    }
}

#[async_trait]
impl ResourceProvider for SearchProvider {
    fn source(&self) -> ResourceSource {
        ResourceSource {
            id: self.source.clone(),
            types: vec![ResourceType::Mod],
            available: true,
            capabilities: vec![
                SourceCapability::Search,
                SourceCapability::Project,
                SourceCapability::Description,
            ],
            supported_search_sorts: vec![ResourceSearchSort::Relevance],
        }
    }
    fn namespace(&self) -> String {
        format!("search-test:{}", self.source.as_str())
    }
    fn batch_limit(&self, _: &ProviderQuery) -> usize {
        100
    }
    async fn fetch_many(
        &self,
        queries: &[ProviderQuery],
    ) -> Vec<ResourceResult<ProviderValue>> {
        self.calls.lock().unwrap().extend_from_slice(queries);
        queries
            .iter()
            .map(|query| {
                if self.fail.load(Ordering::Acquire) {
                    return Err(ResourceError::Network { status: Some(503) });
                }
                match query {
                    ProviderQuery::Search { query, cursor } => {
                        Ok(ProviderValue::Search(slice_page(
                            self.entries.clone(),
                            offset(cursor)?,
                            query.page_size,
                        )?))
                    }
                    ProviderQuery::Project { id, .. } => self
                        .entries
                        .iter()
                        .find(|entry| entry.project.project_id == *id)
                        .map(|metadata| {
                            ProviderValue::Project(ProjectDocument {
                                metadata: metadata.clone(),
                                description: ResourceText {
                                    source: self.source.clone(),
                                    format: TextFormat::Plain,
                                    body: "description".into(),
                                },
                                protocol: Some(json!({"id": id.as_str()})),
                            })
                        })
                        .ok_or(ResourceError::Unavailable),
                    ProviderQuery::Description { .. } => {
                        Ok(ProviderValue::Description(ResourceText {
                            source: self.source.clone(),
                            format: TextFormat::Plain,
                            body: self.source.as_str().into(),
                        }))
                    }
                    _ => Err(ResourceError::Unsupported("test query".into())),
                }
            })
            .collect()
    }
}

fn query(page_size: u32) -> ResourceSearchQuery {
    ResourceSearchQuery {
        filter: None,
        text: String::new(),
        types: vec![ResourceType::Mod],
        sources: vec![],
        target: ResourceTarget {
            game_versions: vec![],
            loaders: vec![],
            channels: vec![],
        },
        categories: vec![],
        sort: ResourceSearchSort::Relevance,
        page_size,
    }
}
async fn main_pool() -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn search_keeps_all_provider_remainders_and_has_no_fabricated_mixed_total()
 {
    let (_directory, cache) = cache().await;
    let mr = SearchProvider::new(
        SourceId::modrinth(),
        &(0..31)
            .map(|index| format!("MR {index}"))
            .collect::<Vec<_>>(),
    );
    let cf = SearchProvider::new(
        SourceId::curseforge(),
        &(0..27)
            .map(|index| format!("CF {index}"))
            .collect::<Vec<_>>(),
    );
    let service =
        ResourceService::new(cache.clone(), vec![mr.clone(), cf.clone()])
            .unwrap();
    let cancel = CancellationToken::new();
    let mut cursor = None;
    let mut returned = std::collections::HashSet::new();
    loop {
        let before = [
            mr.calls.lock().unwrap().len(),
            cf.calls.lock().unwrap().len(),
        ];
        let page = service
            .search(
                query(10),
                cursor,
                RequestId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(page.total.precision, TotalPrecision::Unknown);
        assert_eq!(page.total.value, None);
        for entry in page.entries {
            assert!(returned.insert(entry.entry.default_project));
        }
        assert!(mr.calls.lock().unwrap().len() - before[0] <= 2);
        assert!(cf.calls.lock().unwrap().len() - before[1] <= 2);
        if !page.has_more {
            break;
        }
        cursor = page.cursor;
    }
    assert_eq!(returned.len(), 58);
    assert!(
        service
            .search(
                query(11),
                Some("invented".into()),
                RequestId::new("invalid").unwrap(),
                &cancel
            )
            .await
            .is_err()
    );
    cache.close().await;
}

#[tokio::test]
async fn search_candidates_preserve_ambiguity_variants_and_do_not_create_business_ids()
 {
    let pool = main_pool().await;
    let (_directory, cache) = cache().await;
    let mr = SearchProvider::new(
        SourceId::modrinth(),
        &[
            "Shared".into(),
            "Shared".into(),
            "Sodium Lite".into(),
            "Cafe\u{301}".into(),
        ],
    );
    let cf = SearchProvider::new(
        SourceId::curseforge(),
        &["Shared".into(), "Sodium".into(), "Café".into()],
    );
    let service = ResourceService::with_store(
        cache.clone(),
        vec![mr, cf],
        crate::state::resources::ResourceStore::new(pool.clone()),
    )
    .unwrap();
    let page = service
        .search(
            query(20),
            None,
            RequestId::new("candidates").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 6);
    let accent = page
        .entries
        .iter()
        .find(|entry| entry.projects.len() == 2)
        .unwrap();
    assert_eq!(accent.entry.status, IdentityStatus::Unconfirmed);
    assert_eq!(accent.entry.basis, vec![CandidateBasis::ExactTitle]);
    assert!(
        page.entries
            .iter()
            .filter(|entry| entry.projects[0].title == "Shared")
            .all(|entry| entry.projects.len() == 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM resources")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let mut changed = query(20);
    changed.text = "another query".into();
    if let Some(cursor) = page.cursor {
        assert!(
            service
                .search(
                    changed,
                    Some(cursor),
                    RequestId::new("changed").unwrap(),
                    &CancellationToken::new()
                )
                .await
                .is_err()
        );
    }
    cache.close().await;
}

#[tokio::test]
async fn search_retains_success_when_one_source_fails_and_cancel_does_not_commit_cursor()
 {
    let (_directory, cache) = cache().await;
    let mr = SearchProvider::new(SourceId::modrinth(), &["Working".into()]);
    let cf = SearchProvider::new(SourceId::curseforge(), &["Failed".into()]);
    cf.fail.store(true, Ordering::Release);
    let service = ResourceService::new(cache.clone(), vec![mr, cf]).unwrap();
    let page = service
        .search(
            query(10),
            None,
            RequestId::new("partial").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.source_failures.len(), 1);
    let canceled = CancellationToken::new();
    canceled.cancel();
    assert_eq!(
        service
            .search(
                query(10),
                None,
                RequestId::new("canceled").unwrap(),
                &canceled
            )
            .await
            .unwrap_err(),
        ResourceError::Canceled
    );
    cache.close().await;
}

fn artifact(project: SourceProjectRef) -> ResourceArtifact {
    let release = SourceReleaseRef {
        release_id: SourceReleaseId::new(
            if project.source == SourceId::curseforge() {
                "10"
            } else {
                "release1"
            },
        )
        .unwrap(),
        project,
    };
    ResourceArtifact {
        reference: SourceArtifactRef {
            artifact_id: SourceArtifactId::new("file.jar").unwrap(),
            release,
        },
        file_name: Some("file.jar".into()),
        role: ArtifactRole::Primary,
        available: Some(true),
        integrity: ArtifactIntegrity {
            size: Some(100),
            hashes: vec![ContentHash {
                algorithm: HashAlgorithm::Sha1,
                value: "a".repeat(40),
            }],
        },
        acquisition: ArtifactAcquisition::PublicDownload {
            urls: vec!["https://example.com/file.jar".into()],
        },
        dependencies: vec![],
    }
}

#[tokio::test]
async fn confirmed_favorites_preferences_and_descriptions_survive_cache_clear()
{
    let pool = main_pool().await;
    let (_directory, cache) = cache().await;
    let mr = SearchProvider::new(
        SourceId::modrinth(),
        &["Different MR title".into()],
    );
    let cf = SearchProvider::new(
        SourceId::curseforge(),
        &["Different CF title".into()],
    );
    let left = mr.entries[0].clone();
    let right = cf.entries[0].clone();
    let store = crate::state::resources::ResourceStore::new(pool.clone());
    let service = ResourceService::with_store(
        cache.clone(),
        vec![mr.clone(), cf.clone()],
        store.clone(),
    )
    .unwrap();
    service
        .favorite_add(&pool, left.project.clone(), ResourceType::Mod)
        .await
        .unwrap();
    service
        .favorite_add(&pool, right.project.clone(), ResourceType::Mod)
        .await
        .unwrap();
    sqlx::query("UPDATE resource_favorites SET saved_at = 123")
        .execute(&pool)
        .await
        .unwrap();
    store
        .confirm_projects(
            &left,
            &right,
            &artifact(left.project.clone()),
            &artifact(right.project.clone()),
        )
        .await
        .unwrap();
    store
        .set_preferred_source(&SourceId::curseforge())
        .await
        .unwrap();
    cache.clear().await.unwrap();
    let favorites = service.favorites(&pool).await.unwrap();
    assert_eq!(favorites.len(), 1);
    assert_eq!(favorites[0].saved_at, 123);
    assert_eq!(favorites[0].projects.len(), 2);
    assert_eq!(favorites[0].project.source, SourceId::curseforge());
    let reference = ResourceRef::Source {
        project: left.project.clone(),
    };
    let resource = service
        .get(&reference, None, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(resource.default_project.source, SourceId::curseforge());
    let description = service
        .description(
            &reference,
            &SourceId::modrinth(),
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(description.source, SourceId::modrinth());
    assert_eq!(
        service
            .get(&reference, None, None, &CancellationToken::new())
            .await
            .unwrap()
            .default_project,
        resource.default_project
    );
    let page = service
        .search(
            query(10),
            None,
            RequestId::new("confirmed").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].entry.status, IdentityStatus::Confirmed);
    cf.fail.store(true, Ordering::Release);
    cache.clear().await.unwrap();
    assert_eq!(
        service
            .get(&reference, None, None, &CancellationToken::new())
            .await
            .unwrap()
            .default_project
            .source,
        SourceId::modrinth()
    );
    service
        .favorite_remove(&pool, &right.project)
        .await
        .unwrap();
    assert!(service.favorites(&pool).await.unwrap().is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM content_favorites")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    cache.close().await;
}

#[tokio::test]
async fn connector_favorites_use_stable_ids_without_the_legacy_provider_constraint()
 {
    let pool = main_pool().await;
    let (_directory, cache) = cache().await;
    let store = crate::state::resources::ResourceStore::new(pool.clone());
    let service =
        ResourceService::with_store(cache.clone(), vec![], store).unwrap();
    let project = SourceProjectRef {
        source: SourceId::planet_minecraft(),
        project_id: SourceProjectId::new("connector-project").unwrap(),
    };
    service
        .favorite_add(&pool, project.clone(), ResourceType::Mod)
        .await
        .unwrap();
    assert_eq!(service.favorites(&pool).await.unwrap()[0].project, project);
    service.favorite_remove(&pool, &project).await.unwrap();
    assert!(service.favorites(&pool).await.unwrap().is_empty());
    cache.close().await;
}

#[tokio::test]
async fn long_search_chains_retry_failed_sources_without_losing_their_cursor() {
    let (_directory, cache) = cache().await;
    let mr = SearchProvider::new(
        SourceId::modrinth(),
        &(0..150)
            .map(|index| format!("MR {index}"))
            .collect::<Vec<_>>(),
    );
    let cf =
        SearchProvider::new(SourceId::curseforge(), &["CF recovery".into()]);
    cf.fail.store(true, Ordering::Release);
    let service =
        ResourceService::new(cache.clone(), vec![mr.clone(), cf.clone()])
            .unwrap();
    let mut cursor = None;
    let mut returned = std::collections::HashSet::new();
    let cancel = CancellationToken::new();
    for index in 0..151 {
        if index == 1 {
            cf.fail.store(false, Ordering::Release);
        }
        let page = service
            .search(
                query(1),
                cursor,
                RequestId::new(format!("page-{index}")).unwrap(),
                &cancel,
            )
            .await
            .unwrap();
        if index == 0 {
            assert_eq!(page.source_failures.len(), 1);
        }
        for entry in page.entries {
            assert!(returned.insert(entry.entry.default_project));
        }
        if !page.has_more {
            break;
        }
        cursor = page.cursor;
    }
    assert_eq!(returned.len(), 151);
    assert!(service.searches.len() <= 2);
    cache.close().await;
}

#[tokio::test]
async fn modrinth_boolean_filters_preserve_live_v3_shapes_and_unsupported_sources_report_difference()
 {
    let transport = FixtureTransport::new(|_, request| {
        assert_eq!(request.path, "/v3/search");
        let filters = &request
            .query
            .iter()
            .find(|(key, _)| key == "new_filters")
            .unwrap()
            .1;
        assert!(filters.contains("environment = \"server_only\""));
        assert!(filters.contains("license = \"MIT\""));
        assert!(filters.contains("categories != \"unofficial\""));
        assert!(!request.query.iter().any(|(key, _)| key == "facets"));
        Ok(
            json!({"hits":[{"project_id":"abcdefgh","slug":"sample","name":"Sample","summary":"Summary","project_types":["mod"],"all_project_types":["mod"],"environment":["server_only"],"loaders":["neoforge"],"game_versions":["1.21.1"],"categories":["technology"],"license":"MIT"}],"page":2,"hits_per_page":1,"total_hits":10}),
        )
    });
    let mut search = query(1);
    search.filter = Some(ResourceSearchExpression::All {
        terms: vec![
            ResourceSearchExpression::Predicate {
                field: "environment".into(),
                values: vec!["server_only".into()],
                excluded: false,
            },
            ResourceSearchExpression::Predicate {
                field: "license".into(),
                values: vec!["MIT".into()],
                excluded: false,
            },
            ResourceSearchExpression::Predicate {
                field: "category".into(),
                values: vec![
                    "modrinth:unofficial".into(),
                    "curseforge:10".into(),
                ],
                excluded: true,
            },
        ],
    });
    let result = ModrinthProvider::new(transport)
        .fetch_many(&[ProviderQuery::Search {
            query: search.clone(),
            cursor: Some("1".into()),
        }])
        .await
        .remove(0)
        .unwrap();
    let ProviderValue::Search(page) = result else {
        panic!("wrong search")
    };
    assert_eq!(page.entries[0].environments, vec!["server_only"]);
    assert_eq!(page.entries[0].types, vec![ResourceType::Mod]);
    assert_eq!(page.entries[0].loaders, vec!["neoforge"]);
    let cf = CurseForgeProvider::new(FixtureTransport::new(|_, _| {
        panic!("unsupported filters must not request a broadened search")
    }));
    assert!(matches!(
        cf.fetch_many(&[ProviderQuery::Search {
            query: search,
            cursor: None
        }])
        .await
        .remove(0),
        Err(ResourceError::Unsupported(_))
    ));
}

#[tokio::test]
async fn cached_only_projection_keeps_expired_metadata_without_requesting_a_source()
 {
    let (_directory, cache) = cache().await;
    let provider = MockProvider::new(100, None);
    let service =
        ResourceService::new(cache.clone(), vec![provider.clone()]).unwrap();
    let query = project_query("abcdefgh");
    let key =
        CacheKey::new("mock-v1".into(), SourceId::modrinth(), &query).unwrap();
    cache
        .put(
            &key,
            &project_value(SourceId::modrinth(), "abcdefgh", "Expired"),
            -1,
            cache.generation().await,
            chrono::Utc::now().timestamp_micros(),
        )
        .await;
    assert!(
        service
            .query(
                &SourceId::modrinth(),
                query.clone(),
                CachePolicy::OnlyCached,
                &CancellationToken::new()
            )
            .await
            .unwrap()
            .from_cache
    );
    assert_eq!(provider.calls(), 0);
    assert!(
        service
            .query(
                &SourceId::modrinth(),
                project_query("missing1"),
                CachePolicy::OnlyCached,
                &CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(provider.calls(), 0);
    service
        .query(
            &SourceId::modrinth(),
            query.clone(),
            CachePolicy::UseFresh,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    service
        .query(
            &SourceId::modrinth(),
            query,
            CachePolicy::ForceRefresh,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(provider.calls(), 2);
    cache.close().await;
}

#[tokio::test]
async fn unsupported_content_type_is_reported_for_explicit_and_combined_connector_search()
 {
    let (_directory, cache) = cache().await;
    let transport = FixtureTransport::new(|_, _| {
        panic!("unsupported filter must not broaden the query")
    });
    let connector =
        Arc::new(Available(Arc::new(PlanetMinecraftProvider::new(transport))));
    let mr = SearchProvider::new(SourceId::modrinth(), &["Working".into()]);
    let service =
        ResourceService::new(cache.clone(), vec![mr, connector]).unwrap();
    let page = service
        .search(
            query(10),
            None,
            RequestId::new("combined").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.source_failures.len(), 1);
    let mut explicit = query(10);
    explicit.sources = vec![SourceId::planet_minecraft()];
    let page = service
        .search(
            explicit,
            None,
            RequestId::new("explicit").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(page.entries.is_empty());
    assert_eq!(page.source_failures.len(), 1);
    cache.close().await;
}

#[test]
fn compatibility_projection_does_not_cache_credentials_or_signed_download_urls()
{
    let cleaned = protocol_projection(
        json!({"apiKey":"private", "downloadUrl":"https://example.com/file?token=private", "nested":{"headers":{"authorization":"private"},"url":"https://example.com/public"}}),
    );
    assert!(cleaned.get("apiKey").is_none());
    assert!(cleaned["downloadUrl"].is_null());
    assert!(cleaned["nested"].get("headers").is_none());
    assert_eq!(cleaned["nested"]["url"], "https://example.com/public");
}

#[tokio::test]
async fn v3_project_display_projection_keeps_server_fields_in_the_independent_cache()
 {
    let transport = FixtureTransport::new(|_, request| {
        assert_eq!(request.path, "/v3/projects");
        Ok(
            json!([{"id":"abcdefgh","slug":"sample","name":"Sample","summary":"Summary","body":"Body","project_types":["minecraft_java_server"],"minecraft_java_server":{"address":"example.com"},"apiKey":"secret"}]),
        )
    });
    let (_directory, cache) = cache().await;
    let service = ResourceService::new(
        cache.clone(),
        vec![Arc::new(ModrinthProvider::new(transport.clone()))],
    )
    .unwrap();
    let queries = vec![ProviderQuery::ProjectProtocol {
        locator: "abcdefgh".into(),
        version: "v3".into(),
    }];
    let first = service
        .presentation_many(
            &SourceId::modrinth(),
            queries.clone(),
            &CancellationToken::new(),
        )
        .await
        .remove(0)
        .unwrap();
    let ProviderValue::Project(document) = first.value else {
        panic!("wrong protocol")
    };
    assert_eq!(
        document.protocol.as_ref().unwrap()["minecraft_java_server"]["address"],
        "example.com"
    );
    assert!(document.protocol.as_ref().unwrap().get("apiKey").is_none());
    assert!(
        service
            .presentation_many(
                &SourceId::modrinth(),
                queries,
                &CancellationToken::new()
            )
            .await
            .remove(0)
            .unwrap()
            .from_cache
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    cache.close().await;
}

#[tokio::test]
async fn eight_character_slug_links_use_locator_and_cached_reads_stay_offline()
{
    let transport = FixtureTransport::new(|_, request| {
        assert_eq!(request.path, "/project/eightchr");
        Ok(
            json!({"id":"abcdefgh","slug":"eightchr","title":"Sample","description":"Summary","body":"Body","project_type":"mod"}),
        )
    });
    let (_directory, cache) = cache().await;
    let service = ResourceService::new(
        cache.clone(),
        vec![Arc::new(ModrinthProvider::new(transport.clone()))],
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let project = service
        .locate(&SourceId::modrinth(), "eightchr".into(), &cancel)
        .await
        .unwrap();
    assert_eq!(project.metadata.project.project_id.as_str(), "abcdefgh");
    assert_eq!(
        service
            .locate_with_policy(
                &SourceId::modrinth(),
                "eightchr".into(),
                &cancel,
                CachePolicy::OnlyCached
            )
            .await
            .unwrap(),
        project
    );
    assert!(
        service
            .locate_with_policy(
                &SourceId::modrinth(),
                "missing8".into(),
                &cancel,
                CachePolicy::OnlyCached
            )
            .await
            .is_err()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    cache.close().await;
}

#[tokio::test]
async fn all_sources_skip_archive_when_instance_filters_are_not_applicable() {
    let (_directory, cache) = cache().await;
    let archive =
        Arc::new(McArchiveProvider::new(FixtureTransport::new(|_, _| {
            panic!("inapplicable archive search must not perform a request")
        })));
    let mr = SearchProvider::new(SourceId::modrinth(), &["Compatible".into()]);
    let service =
        ResourceService::new(cache.clone(), vec![mr, archive]).unwrap();
    let mut query = query(10);
    query.target.loaders = vec!["forge".into()];
    let page = service
        .search(
            query.clone(),
            None,
            RequestId::new("all-with-loader").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(page.source_failures.is_empty());
    query.sources = vec![SourceId::mcarchive()];
    let page = service
        .search(
            query,
            None,
            RequestId::new("archive-with-loader").unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(page.entries.is_empty());
    assert_eq!(page.source_failures.len(), 1);
    cache.close().await;
}
