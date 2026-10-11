use super::*;
use tokio_util::sync::CancellationToken;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUpdateCheck {
    pub entries: Vec<InstalledEntryId>,
    pub releases: std::collections::HashMap<InstalledEntryId, SourceReleaseRef>,
    pub unresolved: Vec<String>,
    pub failures: Vec<String>,
    pub failed_entries: Vec<InstalledEntryId>,
}

pub async fn check(
    instance_id: String,
    cancel: &CancellationToken,
) -> ResourceResult<ResourceUpdateCheck> {
    let state = crate::State::get().await?;
    let entries = installed::hydrate(&state, &instance_id, cancel).await?;
    let instance = crate::state::get_instance(&instance_id, &state.pool)
        .await?
        .ok_or_else(|| {
            ResourceError::InvalidRequest("unknown instance".into())
        })?;
    let service = state.resources().await?;
    let target = ipc::ResourceTarget {
        game_versions: vec![instance.applied_content_set.game_version],
        loaders: vec![instance.applied_content_set.loader.as_str().into()],
        channels: plans::update_channels(instance.instance.update_channel),
    };
    let preferred = service.preferred_source().await?;
    let mut result = ResourceUpdateCheck {
        entries: vec![],
        releases: std::collections::HashMap::new(),
        unresolved: vec![],
        failures: vec![],
        failed_entries: vec![],
    };
    for entry in entries.into_iter().filter(|entry| !entry.missing) {
        if cancel.is_cancelled() {
            return Err(ResourceError::Canceled);
        }
        let Some(artifact) = &entry.artifact else {
            result.unresolved.push(entry.relative_path);
            continue;
        };
        let selection = ResourceSelection {
            id: SelectionId::new(entry.entry_id.as_str())?,
            resource: entry
                .resource_id
                .map(|id| ResourceRef::Resource { id })
                .unwrap_or_else(|| ResourceRef::Source {
                    project: artifact.release.project.clone(),
                }),
            release: ReleaseSelection::Automatic,
            resource_type: entry.resource_type,
            scope: entry.scope,
            entry_id: Some(entry.entry_id.clone()),
        };
        let current =
            match service.release(&artifact.release, None, cancel).await {
                Ok(current) => current,
                Err(ResourceError::Canceled) => {
                    return Err(ResourceError::Canceled);
                }
                Err(error) => {
                    result.failed_entries.push(entry.entry_id.clone());
                    result
                        .failures
                        .push(format!("{}: {error}", entry.relative_path));
                    continue;
                }
            };
        match service
            .selected_release_for_current(
                &selection,
                &target,
                &preferred,
                cancel,
                Some(&current),
            )
            .await
        {
            Ok((release, target, _)) => {
                if identity::exact_integrity(
                    &entry.integrity,
                    &target.integrity,
                ) {
                    continue;
                }
                match service.release(&artifact.release, None, cancel).await {
                    Ok(current) if release_is_newer(&release, &current) => {
                        result
                            .releases
                            .insert(entry.entry_id.clone(), release.reference);
                        result.entries.push(entry.entry_id)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        result.failed_entries.push(entry.entry_id.clone());
                        result
                            .failures
                            .push(format!("{}: {error}", entry.relative_path));
                    }
                }
            }
            Err(ResourceError::Canceled) => {
                return Err(ResourceError::Canceled);
            }
            Err(error) => {
                result.failed_entries.push(entry.entry_id.clone());
                result
                    .failures
                    .push(format!("{}: {error}", entry.relative_path));
            }
        }
    }
    Ok(result)
}
