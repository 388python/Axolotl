use super::{identity::LocalArtifactIdentity, *};
use tokio_util::sync::CancellationToken;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledArtifactRecord {
    #[serde(flatten)]
    pub artifact: ResourceArtifact,
    #[serde(default)]
    pub declared_compatibility: Option<ReleaseCompatibility>,
}

pub(crate) async fn attach_compatibility(
    pool: &sqlx::SqlitePool,
    instance_id: &str,
    game_version: &str,
    loader: &str,
    items: &mut [crate::state::instances::InstanceContentSnapshotItem],
) -> crate::Result<()> {
    let rows: Vec<(String, String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT entry.id, file.sha1, file.size, file.local_mod_data, exact.artifact FROM instance_content_entries entry JOIN instance_files file ON file.id = entry.file_id LEFT JOIN resource_installed_artifacts exact ON exact.entry_id = entry.id WHERE entry.instance_id = ? AND file.missing = 0"
    ).bind(instance_id).fetch_all(pool).await?;
    let target = ipc::ResourceTarget {
        game_versions: vec![game_version.into()],
        loaders: vec![loader.into()],
        channels: vec![],
    };
    let indices: std::collections::HashMap<_, _> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.entry_id.clone().map(|id| (id, index)))
        .collect();
    for (id, sha1, size, local, artifact) in rows {
        let Some(&index) = indices.get(&id) else {
            continue;
        };
        let item = &mut items[index];
        if let Some(record) = artifact.as_ref().and_then(|payload| {
            serde_json::from_str::<InstalledArtifactRecord>(payload).ok()
        }) {
            let local = ArtifactIntegrity {
                size: u64::try_from(size).ok(),
                hashes: vec![ContentHash {
                    algorithm: HashAlgorithm::Sha1,
                    value: sha1.clone(),
                }],
            };
            if super::identity::exact_integrity(
                &local,
                &record.artifact.integrity,
            ) {
                item.artifact_ref = Some(record.artifact.reference.clone());
                if !record.artifact.integrity.hashes.is_empty()
                    && item.content.is_some()
                    && item.materialization_state
                        == crate::state::PackMemberMaterializationState::Present
                {
                    item.capabilities.can_change_version = true;
                }
                if item.resource_id.is_none() {
                    item.resource_id =
                        crate::state::resources::ResourceStore::new(
                            pool.clone(),
                        )
                        .lookup(&record.artifact.reference.release.project)
                        .await
                        .map_err(|error| {
                            crate::ErrorKind::OtherError(error.to_string())
                        })?;
                }
            }
        }
        let kind = match item.project_type {
            crate::state::ProjectType::Mod => ResourceType::Mod,
            crate::state::ProjectType::ResourcePack => {
                ResourceType::ResourcePack
            }
            crate::state::ProjectType::ShaderPack => ResourceType::Shader,
            crate::state::ProjectType::DataPack => ResourceType::DataPack,
            _ => continue,
        };
        item.compatibility = None;
        let exact = artifact
            .and_then(|artifact| {
                serde_json::from_str::<InstalledArtifactRecord>(&artifact).ok()
            })
            .filter(|artifact| {
                artifact.artifact.integrity.size == u64::try_from(size).ok()
                    && artifact.artifact.integrity.hashes.iter().any(|hash| {
                        hash.algorithm == HashAlgorithm::Sha1
                            && hash.value.eq_ignore_ascii_case(&sha1)
                    })
            });
        let declared = exact
            .and_then(|record| record.declared_compatibility)
            .or_else(|| {
                let metadata = serde_json::from_str::<
                    crate::mod_metadata::LocalModMetadata,
                >(local.as_deref()?)
                .ok()?;
                Some(ReleaseCompatibility {
                    game_versions: metadata
                        .minecraft_version
                        .into_iter()
                        .collect(),
                    loaders: metadata.loader.into_iter().collect(),
                })
            });
        if let Some(declared) = declared {
            item.compatibility =
                Some(assess_declared_compatibility(&declared, &target, kind));
        }
    }
    Ok(())
}

pub(crate) async fn hydrate(
    state: &crate::State,
    instance_id: &str,
    cancel: &CancellationToken,
) -> ResourceResult<Vec<InstalledResource>> {
    Ok(hydrate_with_failures(state, instance_id, cancel)
        .await?
        .entries)
}

pub(crate) struct HydratedInstalled {
    pub entries: Vec<InstalledResource>,
    pub failures: Vec<(InstalledEntryId, String)>,
}

pub(crate) async fn hydrate_with_failures(
    state: &crate::State,
    instance_id: &str,
    cancel: &CancellationToken,
) -> ResourceResult<HydratedInstalled> {
    let service = state.resources().await?;
    let revision = plans::instance_revision(&state.pool, instance_id).await?;
    let scope = crate::state::instances::commands::resolve_content_scope(
        instance_id,
        None,
        state,
    )
    .await?;
    let root = crate::state::instances::commands::instance_content_root(
        &state.directories,
        &scope.instance,
    )
    .map_err(|error| ResourceError::InvalidRequest(error.to_string()))?;
    let mut entries = plans::installed(&state.pool, instance_id).await?;
    let mut identities = Vec::new();
    let mut indices = Vec::new();
    let mut failures = Vec::new();
    let mut persisted = Vec::new();
    let mut verified = std::collections::HashMap::new();
    for (index, entry) in entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.missing)
    {
        if cancel.is_cancelled() {
            return Err(ResourceError::Canceled);
        }
        let bytes =
            match super::execution::checked_path(&root, &entry.relative_path) {
                Ok(path) => tokio::fs::read(path)
                    .await
                    .map(bytes::Bytes::from)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push((
                    entry.entry_id.clone(),
                    format!("{}: {error}", entry.relative_path),
                ));
                continue;
            }
        };
        let mut hashes = Vec::new();
        use sha2::Digest;
        hashes.push(ContentHash {
            algorithm: HashAlgorithm::Sha512,
            value: format!("{:x}", sha2::Sha512::digest(&bytes)),
        });
        hashes.push(ContentHash {
            algorithm: HashAlgorithm::Sha256,
            value: format!("{:x}", sha2::Sha256::digest(&bytes)),
        });
        let sha1 = crate::util::fetch::sha1_async(bytes.clone()).await?;
        if entry.integrity.size != Some(bytes.len() as u64)
            || !entry.integrity.hashes.iter().any(|hash| {
                hash.algorithm == HashAlgorithm::Sha1
                    && hash.value.eq_ignore_ascii_case(&sha1)
            })
        {
            failures.push((
                entry.entry_id.clone(),
                format!(
                    "{}: installed files changed; refresh content first",
                    entry.relative_path
                ),
            ));
            continue;
        }
        hashes.push(ContentHash {
            algorithm: HashAlgorithm::Sha1,
            value: sha1,
        });
        let saved: Option<String> = sqlx::query_scalar("SELECT artifact FROM resource_installed_artifacts WHERE entry_id = ?").bind(entry.entry_id.as_str()).fetch_optional(&state.pool).await.map_err(plans::db)?;
        if let Some(artifact) = saved
            .and_then(|value| {
                serde_json::from_str::<ResourceArtifact>(&value).ok()
            })
            .filter(|artifact| {
                super::valid_artifact(artifact)
                    && identity::exact_integrity(
                        &ArtifactIntegrity {
                            size: Some(bytes.len() as u64),
                            hashes: hashes.clone(),
                        },
                        &artifact.integrity,
                    )
            })
        {
            verified.insert(index, artifact);
        }
        identities.push(LocalArtifactIdentity {
            integrity: ArtifactIntegrity {
                size: Some(bytes.len() as u64),
                hashes,
            },
            fingerprint: Some(crate::api::curseforge::compute_fingerprint(
                &bytes,
            )),
        });
        indices.push(index);
    }
    let sources: Vec<_> = service
        .sources()
        .into_iter()
        .filter(|source| {
            source.available
                && matches!(source.id.as_str(), "modrinth" | "curseforge")
        })
        .map(|source| source.id)
        .collect();
    let recognized = service
        .identify_many(&sources, &identities, CachePolicy::UseFresh, cancel)
        .await;
    for ((index, local), recognition) in
        indices.into_iter().zip(identities).zip(recognized)
    {
        let entry = &mut entries[index];
        let refs = crate::state::instances::adapters::sqlite::content_rows::get_content_provider_refs(entry.entry_id.as_str(), &state.pool).await?;
        let origin = crate::state::instances::adapters::sqlite::content_rows::get_content_origin_provider(entry.entry_id.as_str(), &state.pool).await?;
        let mut matches = recognition.matches;
        if let Some(artifact) = verified.remove(&index) {
            if !matches
                .iter()
                .any(|other| other.reference == artifact.reference)
            {
                matches.push(artifact);
            }
        }
        // Exact legacy release references can corroborate files absent from a
        // hash index. They must agree with current local bytes.
        for reference in &refs {
            if let Some(release_id) = reference.database_release_id() {
                let release_ref = SourceReleaseRef {
                    project: SourceProjectRef {
                        source: SourceId::new(reference.provider().as_str())?,
                        project_id: SourceProjectId::new(
                            reference.database_project_id(),
                        )?,
                    },
                    release_id: SourceReleaseId::new(release_id)?,
                };
                if matches
                    .iter()
                    .any(|artifact| artifact.reference.release == release_ref)
                {
                    continue;
                }
                if let Ok(release) =
                    service.release(&release_ref, None, cancel).await
                {
                    for artifact in
                        release.artifacts.into_iter().filter(|artifact| {
                            artifact.role == ArtifactRole::Primary
                                && identity::exact_integrity(
                                    &local.integrity,
                                    &artifact.integrity,
                                )
                        })
                    {
                        if !matches
                            .iter()
                            .any(|other| other.reference == artifact.reference)
                        {
                            matches.push(artifact);
                        }
                    }
                }
            }
        }
        if cancel.is_cancelled() {
            return Err(ResourceError::Canceled);
        }
        let selected = entry
            .artifact
            .as_ref()
            .and_then(|reference| {
                matches
                    .iter()
                    .find(|artifact| &artifact.reference == reference)
            })
            .or_else(|| {
                let origin_matches: Vec<_> = matches
                    .iter()
                    .filter(|artifact| {
                        origin.is_some_and(|origin| {
                            artifact.reference.release.project.source.as_str()
                                == origin.as_str()
                        })
                    })
                    .collect();
                if origin_matches.len() == 1 {
                    Some(origin_matches[0])
                } else if origin.is_none() && matches.len() == 1 {
                    matches.first()
                } else {
                    None
                }
            })
            .cloned();
        if selected.is_none() {
            entry.artifact = None;
            entry.known_artifacts.clear();
            for (source, error) in recognition.failures {
                failures.push((
                    entry.entry_id.clone(),
                    format!(
                        "{} ({}): {error}",
                        entry.relative_path,
                        source.as_str()
                    ),
                ));
            }
            continue;
        }
        let mut selected = selected.unwrap();
        if !selected
            .integrity
            .hashes
            .iter()
            .any(|hash| hash.algorithm == HashAlgorithm::Sha1)
        {
            selected.integrity.hashes.extend(
                local
                    .integrity
                    .hashes
                    .iter()
                    .filter(|hash| hash.algorithm == HashAlgorithm::Sha1)
                    .cloned(),
            );
        }
        let metadata = match service
            .project_document(&selected.reference.release.project, None, cancel)
            .await
        {
            Ok(document) => document.metadata,
            Err(ResourceError::Canceled) => {
                return Err(ResourceError::Canceled);
            }
            Err(error) => {
                failures.push((
                    entry.entry_id.clone(),
                    format!("{}: {error}", entry.relative_path),
                ));
                entry.artifact = None;
                continue;
            }
        };
        if !metadata.types.contains(&entry.resource_type) {
            entry.artifact = None;
            continue;
        }
        if let Some(store) = service.store() {
            let id = store
                .ensure_project(&metadata.project, &metadata.types)
                .await?;
            entry.resource_id = Some(id);
            for matched in matches.iter().filter(|_| !recognition.ambiguous) {
                if matched.reference.release.project != metadata.project {
                    let other = match service
                        .project_document(
                            &matched.reference.release.project,
                            None,
                            cancel,
                        )
                        .await
                    {
                        Ok(document) => document.metadata,
                        Err(ResourceError::Canceled) => {
                            return Err(ResourceError::Canceled);
                        }
                        Err(_) => continue,
                    };
                    store
                        .confirm_projects(&metadata, &other, &selected, matched)
                        .await?;
                }
            }
            entry.resource_id = store.lookup(&metadata.project).await?;
        }
        entry.artifact = Some(selected.reference.clone());
        entry.known_artifacts = matches
            .iter()
            .filter(|artifact| {
                !recognition.ambiguous
                    || artifact.reference == selected.reference
            })
            .map(|artifact| artifact.reference.clone())
            .collect();
        entry.integrity = local.integrity;
        entry.original_file_name = selected.file_name.clone();
        persisted.push((entry.entry_id.clone(), selected, entry.scope.clone()));
    }
    for (id, _) in &failures {
        if let Some(entry) =
            entries.iter_mut().find(|entry| &entry.entry_id == id)
        {
            entry.artifact = None;
            entry.known_artifacts.clear();
        }
    }
    if cancel.is_cancelled() {
        return Err(ResourceError::Canceled);
    }
    let _writer = state.acquire_install_db_permit().await?;
    let _lock = state.lock_instance_content_exclusive(instance_id).await;
    if plans::instance_revision(&state.pool, instance_id).await? != revision {
        return Err(ResourceError::InvalidRequest(
            "instance changed while verifying installed resources".into(),
        ));
    }
    let mut tx = state.pool.begin().await.map_err(plans::db)?;
    for (id, artifact, scope) in persisted {
        let previous: Option<String> = sqlx::query_scalar("SELECT artifact FROM resource_installed_artifacts WHERE entry_id = ?").bind(id.as_str()).fetch_optional(&mut *tx).await.map_err(plans::db)?;
        let declared_compatibility = previous
            .and_then(|payload| {
                serde_json::from_str::<InstalledArtifactRecord>(&payload).ok()
            })
            .filter(|previous| {
                previous.artifact.reference == artifact.reference
                    && identity::exact_integrity(
                        &previous.artifact.integrity,
                        &artifact.integrity,
                    )
            })
            .and_then(|previous| previous.declared_compatibility);
        let record = InstalledArtifactRecord {
            artifact,
            declared_compatibility,
        };
        sqlx::query("INSERT INTO resource_installed_artifacts(entry_id, artifact, scope) VALUES (?, ?, ?) ON CONFLICT(entry_id) DO UPDATE SET artifact = excluded.artifact, scope = excluded.scope").bind(id.as_str()).bind(serde_json::to_string(&record)?).bind(serde_json::to_string(&scope)?).execute(&mut *tx).await.map_err(plans::db)?;
    }
    tx.commit().await.map_err(plans::db)?;
    Ok(HydratedInstalled { entries, failures })
}

pub async fn list(
    instance_id: String,
    cancel: &CancellationToken,
) -> ResourceResult<Vec<InstalledResource>> {
    let state = crate::State::get().await?;
    hydrate(&state, &instance_id, cancel).await
}

pub async fn prepare_entries(
    instance_id: String,
    intent: ResourceOperationIntent,
    entry_ids: Vec<InstalledEntryId>,
    release: Option<SourceReleaseRef>,
    install_dependencies: bool,
    cancel: &CancellationToken,
) -> ResourceResult<ResourceOperationPlan> {
    if intent == ResourceOperationIntent::Install || entry_ids.is_empty() {
        return Err(ResourceError::InvalidRequest(
            "select installed entries to change".into(),
        ));
    }
    let state = crate::State::get().await?;
    let hydrated = hydrate_with_failures(&state, &instance_id, cancel).await?;
    if let Some((_, error)) = hydrated
        .failures
        .iter()
        .find(|(id, _)| entry_ids.contains(id))
    {
        return Err(ResourceError::InvalidRequest(error.clone()));
    }
    let entries = hydrated.entries;
    let mut selections = Vec::new();
    for id in entry_ids {
        let entry = entries
            .iter()
            .find(|entry| entry.entry_id == id && !entry.missing)
            .ok_or_else(|| {
                ResourceError::InvalidRequest(
                    "installed entry is missing".into(),
                )
            })?;
        let artifact = entry.artifact.as_ref().ok_or_else(|| {
            ResourceError::InvalidRequest(format!(
                "cannot verify the current artifact: {}",
                entry.relative_path
            ))
        })?;
        selections.push(ResourceSelection {
            id: SelectionId::new(id.as_str())?,
            resource: entry
                .resource_id
                .as_ref()
                .map(|id| ResourceRef::Resource { id: id.clone() })
                .unwrap_or_else(|| ResourceRef::Source {
                    project: artifact.release.project.clone(),
                }),
            release: release
                .as_ref()
                .map(|reference| ReleaseSelection::Release {
                    reference: reference.clone(),
                })
                .unwrap_or(ReleaseSelection::Automatic),
            resource_type: entry.resource_type,
            scope: entry.scope.clone(),
            entry_id: Some(id),
        });
    }
    plans::prepare_with_installed(
        instance_id,
        intent,
        selections,
        install_dependencies,
        cancel,
        Some(&entries),
    )
    .await
}
