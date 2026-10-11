use super::{identity::LocalArtifactIdentity, *};
use tokio_util::sync::CancellationToken;

pub(crate) async fn hydrate(
    state: &crate::State,
    instance_id: &str,
    cancel: &CancellationToken,
) -> ResourceResult<Vec<InstalledResource>> {
    let service = state.resources().await?;
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
    for (index, entry) in entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.missing)
    {
        if cancel.is_cancelled() {
            return Err(ResourceError::Canceled);
        }
        let path = super::execution::checked_path(&root, &entry.relative_path)
            .map_err(|error| {
                ResourceError::InvalidRequest(error.to_string())
            })?;
        let bytes = bytes::Bytes::from(tokio::fs::read(path).await.map_err(
            |error| ResourceError::InvalidRequest(error.to_string()),
        )?);
        let mut hashes = entry.integrity.hashes.clone();
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
            || !hashes.iter().any(|hash| {
                hash.algorithm == HashAlgorithm::Sha1
                    && hash.value.eq_ignore_ascii_case(&sha1)
            })
        {
            return Err(ResourceError::InvalidRequest(
                "installed files changed; refresh content first".into(),
            ));
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
        if recognition.ambiguous || selected.is_none() {
            entry.artifact = None;
            entry.known_artifacts.clear();
            continue;
        }
        let selected = selected.unwrap();
        let metadata = service
            .project_document(&selected.reference.release.project, None, cancel)
            .await?
            .metadata;
        if !metadata.types.contains(&entry.resource_type) {
            entry.artifact = None;
            continue;
        }
        if let Some(store) = service.store() {
            let id = store
                .ensure_project(&metadata.project, &metadata.types)
                .await?;
            entry.resource_id = Some(id);
            for matched in &matches {
                if matched.reference.release.project != metadata.project {
                    let other = service
                        .project_document(
                            &matched.reference.release.project,
                            None,
                            cancel,
                        )
                        .await?
                        .metadata;
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
            .map(|artifact| artifact.reference.clone())
            .collect();
        entry.integrity = local.integrity;
        entry.original_file_name = selected.file_name.clone();
        sqlx::query("INSERT INTO resource_installed_artifacts(entry_id, artifact, scope) VALUES (?, ?, ?) ON CONFLICT(entry_id) DO UPDATE SET artifact = excluded.artifact, scope = excluded.scope").bind(entry.entry_id.as_str()).bind(serde_json::to_string(&selected)?).bind(serde_json::to_string(&entry.scope)?).execute(&state.pool).await.map_err(plans::db)?;
    }
    Ok(entries)
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
    let entries = hydrate(&state, &instance_id, cancel).await?;
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
    plans::prepare(
        instance_id,
        intent,
        selections,
        install_dependencies,
        cancel,
    )
    .await
}
