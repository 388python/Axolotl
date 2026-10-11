use chrono::Utc;
use sqlx::Row;

use super::*;

fn db(error: sqlx::Error) -> ResourceError {
    ResourceError::Database(error.to_string())
}

pub async fn list(
    instance_id: String,
    entry_id: Option<InstalledEntryId>,
) -> ResourceResult<Vec<ResourceHistoryEntry>> {
    let state = crate::State::get().await?;
    let rows = if let Some(entry_id) = entry_id {
        sqlx::query("SELECT payload FROM resource_history WHERE instance_id = ? AND entry_id = ? ORDER BY recorded_at DESC, rowid DESC")
            .bind(&instance_id)
            .bind(entry_id.as_str())
            .fetch_all(&state.pool)
            .await
            .map_err(db)?
    } else {
        sqlx::query("SELECT payload FROM resource_history WHERE instance_id = ? ORDER BY recorded_at DESC, rowid DESC")
            .bind(&instance_id)
            .fetch_all(&state.pool)
            .await
            .map_err(db)?
    };
    let scope = crate::state::instances::commands::resolve_content_scope(
        &instance_id,
        None,
        &state,
    )
    .await?;
    let root = crate::state::instances::commands::instance_content_root(
        &state.directories,
        &scope.instance,
    )
    .map_err(|error| ResourceError::InvalidRequest(error.to_string()))?;
    let mut entries = Vec::new();
    for row in rows {
        let mut entry: ResourceHistoryEntry =
            serde_json::from_str(row.get("payload"))?;
        entry.backup_available =
            match (&entry.backup_relative_path, &entry.backup_integrity) {
                (Some(path), Some(integrity)) if entry.backup_available => {
                    super::execution::matches_file(
                        &super::execution::checked_path(&root, path).map_err(
                            |error| {
                                ResourceError::InvalidRequest(error.to_string())
                            },
                        )?,
                        integrity,
                    )
                    .await
                    .unwrap_or(false)
                }
                _ => false,
            };
        entries.push(entry);
    }
    Ok(entries)
}

pub async fn rollback(
    instance_id: String,
    entry_id: InstalledEntryId,
    history_id: HistoryId,
) -> ResourceResult<crate::install::InstallJobSnapshot> {
    let state = crate::State::get().await?;
    let histories = list(instance_id.clone(), Some(entry_id.clone())).await?;
    let history = histories
        .iter()
        .find(|entry| entry.id == history_id && entry.backup_available)
        .ok_or_else(|| {
            ResourceError::InvalidRequest(
                "verified rollback backup not available".into(),
            )
        })?;
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM resource_history WHERE id = ?")
            .bind(history_id.as_str())
            .fetch_one(&state.pool)
            .await
            .map_err(db)?;
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let artifact: ResourceArtifact = serde_json::from_value(
        payload.get("previousArtifact").cloned().ok_or_else(|| {
            ResourceError::InvalidRequest(
                "history lacks exact artifact metadata".into(),
            )
        })?,
    )?;
    let current = plans::installed(&state.pool, &instance_id)
        .await?
        .into_iter()
        .find(|entry| entry.entry_id == entry_id && !entry.missing)
        .ok_or_else(|| {
            ResourceError::InvalidRequest("installed entry disappeared".into())
        })?;
    if !identity::exact_integrity(&current.integrity, &history.target.integrity)
        || current.artifact != history.target.artifact
        || current.relative_path != history.target.relative_path
    {
        return Err(ResourceError::InvalidRequest(
            "installed artifact changed since this history entry".into(),
        ));
    }
    let id = PlanId::new(uuid::Uuid::new_v4().to_string())?;
    let node_id = SelectionId::new(entry_id.as_str())?;
    let reference = current
        .resource_id
        .as_ref()
        .map(|id| ResourceRef::Resource { id: id.clone() })
        .unwrap_or_else(|| ResourceRef::Source {
            project: artifact.reference.release.project.clone(),
        });
    let plan = ResourceOperationPlan {
        schema_version: ResourceContractVersion,
        id: id.clone(),
        revision: 1,
        instance_id: instance_id.clone(),
        instance_revision: plans::instance_revision(&state.pool, &instance_id)
            .await?,
        intent: ResourceOperationIntent::SwitchVersion,
        selections: vec![ResourceSelection {
            id: node_id.clone(),
            resource: reference.clone(),
            release: ReleaseSelection::Artifact {
                reference: artifact.reference.clone(),
            },
            resource_type: current.resource_type,
            scope: current.scope.clone(),
            entry_id: Some(entry_id.clone()),
        }],
        active_roots: vec![node_id.clone()],
        nodes: vec![ResourceDependencyNode {
            id: node_id.clone(),
            resource: reference,
            scope: current.scope.clone(),
            state: DependencyNodeState::Update,
            artifact: Some(artifact.clone()),
            installed_entries: vec![entry_id.clone()],
            compatibility: CompatibilityAssessment {
                game_version: CompatibilityState::Unknown,
                loader: CompatibilityState::Unknown,
                reasons: vec!["restoring previously installed artifact".into()],
            },
        }],
        edges: history.previous_dependencies.clone(),
        actions: vec![ResourceEntryAction {
            entry_id: entry_id.clone(),
            node_id,
            kind: ResourceActionKind::Replace,
            target: artifact,
            target_relative_path: current.relative_path.clone(),
            current: Some(current),
        }],
        issues: vec![],
        exceptions: vec![],
        selection_reasons: vec![],
    };
    let draft = plans::OperationDraft {
        plan: plan.clone(),
        install_dependencies: false,
        candidates: vec![],
        local_files: std::collections::HashMap::from([(
            entry_id,
            history.backup_relative_path.clone().unwrap(),
        )]),
        restore_history: Some(history_id),
    };
    sqlx::query("INSERT INTO resource_operation_plans(id, instance_id, revision, payload) VALUES (?, ?, 1, ?)").bind(id.as_str()).bind(&instance_id).bind(serde_json::to_string(&draft)?).execute(&state.pool).await.map_err(db)?;
    plans::queue(id, 1).await
}

pub(crate) fn history_entry(
    instance_id: &str,
    current: &InstalledResource,
    target: InstalledResource,
    plan: &ResourceOperationPlan,
    backup_relative_path: Option<String>,
) -> ResourceHistoryEntry {
    ResourceHistoryEntry {
        id: HistoryId::new(uuid::Uuid::new_v4().to_string())
            .expect("UUID is a valid history ID"),
        entry_id: current.entry_id.clone(),
        instance_id: instance_id.to_string(),
        recorded_at: Utc::now(),
        previous: current.clone(),
        target,
        previous_nodes: plan.nodes.clone(),
        target_nodes: plan.nodes.clone(),
        previous_dependencies: plan.edges.clone(),
        target_dependencies: plan.edges.clone(),
        backup_integrity: Some(current.integrity.clone()),
        backup_available: backup_relative_path.is_some(),
        backup_relative_path,
    }
}
