use super::{ipc::ResourceTarget, providers::*, *};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OperationDraft {
    pub plan: ResourceOperationPlan,
    pub install_dependencies: bool,
    pub candidates: Vec<ResourceSelection>,
    #[serde(default)]
    pub local_files: HashMap<InstalledEntryId, String>,
    #[serde(default)]
    pub restore_history: Option<HistoryId>,
}

pub(crate) fn db(error: sqlx::Error) -> ResourceError {
    ResourceError::Database(error.to_string())
}
fn invalid(message: impl Into<String>) -> ResourceError {
    ResourceError::InvalidRequest(message.into())
}
fn issue(
    plan: &mut ResourceOperationPlan,
    code: ResourceIssueCode,
    node: &SelectionId,
    blocking: bool,
    message: impl Into<String>,
) {
    plan.issues.push(ResourcePlanIssue {
        code,
        selection_id: Some(node.clone()),
        entry_id: None,
        blocking,
        message: message.into(),
    });
}

fn selection_matches_artifact(
    selection: &ReleaseSelection,
    artifact: Option<&ResourceArtifact>,
) -> bool {
    match selection {
        ReleaseSelection::Automatic => true,
        ReleaseSelection::Release { reference } => artifact
            .is_some_and(|artifact| &artifact.reference.release == reference),
        ReleaseSelection::Artifact { reference } => {
            artifact.is_some_and(|artifact| &artifact.reference == reference)
        }
    }
}

pub fn validate_scope(
    scope: &InstallScope,
    kind: ResourceType,
) -> ResourceResult<()> {
    match (scope, kind) {
        (InstallScope::World { relative_path }, ResourceType::DataPack)
            if safe_relative_path(relative_path)
                && relative_path.starts_with("saves/")
                && relative_path.split('/').count() == 2 =>
        {
            Ok(())
        }
        (
            InstallScope::Instance,
            ResourceType::Mod
            | ResourceType::Plugin
            | ResourceType::Shader
            | ResourceType::ResourcePack,
        ) => Ok(()),
        _ => Err(invalid(
            "this content type requires its supported installation scope",
        )),
    }
}
pub fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | "..")
                && !part.ends_with(['.', ' '])
                && !matches!(
                    part.split('.')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}
pub fn artifact_path(
    scope: &InstallScope,
    kind: ResourceType,
    artifact: &ResourceArtifact,
) -> ResourceResult<String> {
    validate_scope(scope, kind)?;
    let file = artifact
        .file_name
        .as_ref()
        .ok_or_else(|| invalid("artifact has no filename"))?;
    if !safe_relative_path(file) || file.contains('/') {
        return Err(invalid("invalid artifact filename"));
    }
    let base = match scope {
        InstallScope::World { relative_path } => {
            format!("{relative_path}/datapacks")
        }
        InstallScope::Instance => match kind {
            ResourceType::Mod => "mods",
            ResourceType::Plugin => "plugins",
            ResourceType::Shader => "shaderpacks",
            ResourceType::ResourcePack => "resourcepacks",
            _ => return Err(invalid("unsupported content scope")),
        }
        .into(),
    };
    let allowed = if matches!(kind, ResourceType::Mod | ResourceType::Plugin) {
        file.to_ascii_lowercase().ends_with(".jar")
    } else {
        file.to_ascii_lowercase().ends_with(".zip")
    };
    if !allowed {
        return Err(invalid(
            "artifact file type is not installable in this scope",
        ));
    }
    Ok(format!("{base}/{file}"))
}

pub(crate) async fn instance_revision(
    pool: &SqlitePool,
    instance_id: &str,
) -> ResourceResult<u64> {
    let value: Option<i64> = sqlx::query_scalar("SELECT content.revision FROM instances instance JOIN instance_content_sets content ON content.id = instance.applied_content_set_id WHERE instance.id = ?").bind(instance_id).fetch_optional(pool).await.map_err(db)?;
    value
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| invalid("instance has no applied content set"))
}

pub(crate) async fn installed(
    pool: &SqlitePool,
    instance_id: &str,
) -> ResourceResult<Vec<InstalledResource>> {
    let rows = sqlx::query("SELECT entry.id, entry.project_type, entry.ownership_kind, (SELECT member.id FROM instance_pack_members member WHERE member.content_entry_id = entry.id LIMIT 1) AS member_id, file.relative_path, file.enabled, file.missing, file.sha1, file.size, exact.artifact, exact.scope FROM instance_content_entries entry JOIN instance_content_sets content ON content.id = entry.content_set_id JOIN instance_files file ON file.id = entry.file_id LEFT JOIN resource_installed_artifacts exact ON exact.entry_id = entry.id WHERE entry.instance_id = ? AND content.id = (SELECT applied_content_set_id FROM instances WHERE id = entry.instance_id)").bind(instance_id).fetch_all(pool).await.map_err(db)?;
    let mut result = vec![];
    for row in rows {
        let entry_id: String = row.get("id");
        let kind: String = row.get("project_type");
        let resource_type = match kind.as_str() {
            "mod" => ResourceType::Mod,
            "plugin" => ResourceType::Plugin,
            "shader" | "shaderpack" => ResourceType::Shader,
            "resourcepack" => ResourceType::ResourcePack,
            "datapack" => ResourceType::DataPack,
            _ => continue,
        };
        let path: String = row.get("relative_path");
        let exact =
            row.get::<Option<String>, _>("artifact").and_then(|value| {
                serde_json::from_str::<ResourceArtifact>(&value).ok()
            });
        let scope = row
            .get::<Option<String>, _>("scope")
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_else(|| {
                if resource_type == ResourceType::DataPack {
                    InstallScope::World {
                        relative_path: path
                            .split('/')
                            .take(2)
                            .collect::<Vec<_>>()
                            .join("/"),
                    }
                } else {
                    InstallScope::Instance
                }
            });
        let links = sqlx::query("SELECT resource_id, source, project_id, release_id, artifact_id, is_origin FROM resource_entry_links WHERE entry_id = ? ORDER BY is_origin DESC").bind(&entry_id).fetch_all(pool).await.map_err(db)?;
        let mut resource_id = links.first().and_then(|row| {
            ResourceId::new(row.get::<String, _>("resource_id")).ok()
        });
        if resource_id.is_none() {
            let refs = crate::state::instances::adapters::sqlite::content_rows::get_content_provider_refs(&entry_id, pool).await?;
            let origin = crate::state::instances::adapters::sqlite::content_rows::get_content_origin_provider(&entry_id, pool).await?;
            if let Some(reference) = refs
                .iter()
                .find(|reference| Some(reference.provider()) == origin)
                .or_else(|| refs.first())
            {
                let project = SourceProjectRef {
                    source: SourceId::new(reference.provider().as_str())?,
                    project_id: SourceProjectId::new(
                        reference.database_project_id(),
                    )?,
                };
                if crate::state::instances::adapters::sqlite::resource_rows::validate_project(&project).is_ok() {
                    resource_id = Some(crate::state::resources::ResourceStore::new(pool.clone()).ensure_project(&project, &[resource_type]).await?);
                }
            }
        }
        let artifact = exact
            .as_ref()
            .map(|file| file.reference.clone())
            .or_else(|| {
                let link = links.first()?;
                let artifact_id =
                    link.get::<Option<String>, _>("artifact_id")?;
                Some(SourceArtifactRef {
                    release: SourceReleaseRef {
                        project: SourceProjectRef {
                            source: SourceId::new(
                                link.get::<String, _>("source"),
                            )
                            .ok()?,
                            project_id: SourceProjectId::new(
                                link.get::<String, _>("project_id"),
                            )
                            .ok()?,
                        },
                        release_id: SourceReleaseId::new(
                            link.get::<Option<String>, _>("release_id")?,
                        )
                        .ok()?,
                    },
                    artifact_id: SourceArtifactId::new(artifact_id).ok()?,
                })
            });
        let sha1: String = row.get("sha1");
        let size: i64 = row.get("size");
        let ownership: String = row.get("ownership_kind");
        result.push(InstalledResource {
            entry_id: InstalledEntryId::new(entry_id)?,
            instance_id: instance_id.into(),
            resource_id,
            artifact: artifact.clone(),
            known_artifacts: artifact.into_iter().collect(),
            integrity: ArtifactIntegrity {
                size: u64::try_from(size).ok(),
                hashes: vec![ContentHash {
                    algorithm: HashAlgorithm::Sha1,
                    value: sha1,
                }],
            },
            relative_path: path,
            original_file_name: exact.and_then(|file| file.file_name),
            resource_type,
            scope,
            enabled: row.get::<i64, _>("enabled") != 0,
            missing: row.get::<i64, _>("missing") != 0,
            ownership: if ownership == "pack_managed" {
                ResourceOwnership::PackManaged {
                    pack_member_id: row
                        .get::<Option<String>, _>("member_id")
                        .ok_or_else(|| {
                        invalid(
                            "pack-managed entry has no persistent pack member",
                        )
                    })?,
                }
            } else if ownership == "user_added" {
                ResourceOwnership::UserAdded
            } else {
                ResourceOwnership::LocalDiscovered
            },
        });
    }
    Ok(result)
}

impl ResourceService {
    async fn canonical(
        self: &Arc<Self>,
        selection: &mut ResourceSelection,
        cancel: &CancellationToken,
    ) -> ResourceResult<SourceProjectMetadata> {
        validate_scope(&selection.scope, selection.resource_type)?;
        let resource =
            self.get(&selection.resource, None, None, cancel).await?;
        let metadata = resource
            .projects
            .iter()
            .find(|project| project.project == resource.default_project)
            .unwrap()
            .clone();
        if !metadata.types.contains(&selection.resource_type) {
            return Err(invalid(
                "selected project does not provide this content type",
            ));
        }
        let id = if let Some(store) = self.store() {
            Some(
                store
                    .ensure_project(&metadata.project, &metadata.types)
                    .await?,
            )
        } else {
            resource.id
        };
        if let Some(id) = id {
            selection.resource = ResourceRef::Resource { id };
        }
        Ok(metadata)
    }

    pub(crate) async fn selected_release_for_current(
        self: &Arc<Self>,
        selection: &ResourceSelection,
        target: &ResourceTarget,
        preferred: &SourceId,
        cancel: &CancellationToken,
        current: Option<&ResourceRelease>,
    ) -> ResourceResult<(
        ResourceRelease,
        ResourceArtifact,
        ReleaseSelectionBasis,
    )> {
        let mut dependency_target;
        let target = if selection.id.as_str().starts_with("dependency:") {
            dependency_target = target.clone();
            dependency_target.channels.clear();
            &dependency_target
        } else {
            target
        };
        let (_, projects) = self.source_projects(&selection.resource).await?;
        let (release, basis) = match &selection.release {
            ReleaseSelection::Release { reference } => {
                if !projects.contains(&reference.project) {
                    return Err(invalid("release belongs to another resource"));
                }
                (
                    self.release(reference, None, cancel).await?,
                    ReleaseSelectionBasis::FixedByUser,
                )
            }
            ReleaseSelection::Artifact { reference } => {
                if !projects.contains(&reference.release.project) {
                    return Err(invalid(
                        "artifact belongs to another resource",
                    ));
                }
                (
                    self.release(&reference.release, None, cancel).await?,
                    ReleaseSelectionBasis::FixedByUser,
                )
            }
            ReleaseSelection::Automatic => {
                let mut releases = vec![];
                let mut failures = Vec::new();
                let mut catalog_target = target.clone();
                if current.is_none() {
                    catalog_target.channels.clear();
                }
                for project in projects {
                    let mut cursor = None;
                    let mut seen = HashSet::new();
                    loop {
                        let slug = if let Some(store) = self.store() {
                            store.project_alias(&project).await?
                        } else {
                            None
                        };
                        let response = self
                            .query(
                                &project.source,
                                ProviderQuery::Releases {
                                    project_id: project.project_id.clone(),
                                    slug,
                                    target: catalog_target.clone(),
                                    cursor: cursor.clone(),
                                    page_size: 50,
                                },
                                CachePolicy::UseFresh,
                                cancel,
                            )
                            .await;
                        let value = match response {
                            Ok(value) => value,
                            Err(ResourceError::Canceled) => {
                                return Err(ResourceError::Canceled);
                            }
                            Err(error) => {
                                failures.push(format!(
                                    "{}: {error}",
                                    project.source.as_str()
                                ));
                                break;
                            }
                        };
                        let ProviderValue::Releases(page) = value.value else {
                            return Err(invalid(
                                "source returned no release page",
                            ));
                        };
                        releases.extend(page.entries.into_iter().filter(
                            |release| {
                                let compatibility = assess_compatibility(
                                    release,
                                    target,
                                    selection.resource_type,
                                );
                                compatibility.game_version
                                    == CompatibilityState::Compatible
                                    && compatibility.loader
                                        == CompatibilityState::Compatible
                                    && current.is_none_or(|current| {
                                        !release_is_known_lower(
                                            release, current,
                                        )
                                    })
                            },
                        ));
                        cursor = page.cursor;
                        if page.window_exhausted {
                            return Err(ResourceError::Unsupported("release window exhausted; choose an exact release".into()));
                        }
                        match &cursor {
                            Some(cursor)
                                if seen.len() < 100
                                    && seen.insert(cursor.clone()) => {}
                            None => break,
                            _ => {
                                return Err(invalid(
                                    "release cursor repeated or exceeded its window",
                                ));
                            }
                        }
                    }
                }
                let preferred_channels: Vec<_> = releases
                    .iter()
                    .filter(|release| {
                        target.channels.is_empty()
                            || target.channels.contains(&release.channel)
                    })
                    .cloned()
                    .collect();
                if !preferred_channels.is_empty() {
                    releases = preferred_channels;
                }
                select_automatic_release(releases, preferred).ok_or_else(
                    || {
                        invalid(format!(
                            "no compatible release found for {:?} / {:?}{}",
                            target.game_versions,
                            target.loaders,
                            if failures.is_empty() {
                                String::new()
                            } else {
                                format!("; {}", failures.join("; "))
                            }
                        ))
                    },
                )?
            }
        };
        let artifact = match &selection.release {
            ReleaseSelection::Artifact { reference } => release
                .artifacts
                .iter()
                .find(|artifact| &artifact.reference == reference),
            _ => {
                let primary: Vec<_> = release
                    .artifacts
                    .iter()
                    .filter(|artifact| artifact.role == ArtifactRole::Primary)
                    .collect();
                if primary.len() == 1 {
                    Some(primary[0])
                } else {
                    None
                }
            }
        }
        .cloned()
        .ok_or_else(|| invalid("select one exact primary artifact"))?;
        if !valid_artifact(&artifact)
            || artifact.integrity.size.is_none()
            || !artifact
                .integrity
                .hashes
                .iter()
                .any(|hash| hash.algorithm != HashAlgorithm::Md5)
            || artifact.reference.release != release.reference
            || artifact.available == Some(false)
        {
            return Err(invalid("invalid artifact integrity or ownership"));
        }
        Ok((release, artifact, basis))
    }

    async fn dependency_selection(
        self: &Arc<Self>,
        dependency: &ArtifactDependency,
        parent: &ResourceSelection,
        cancel: &CancellationToken,
    ) -> ResourceResult<Option<ResourceSelection>> {
        let (resource, release) = match &dependency.target {
            DependencyTarget::Resource { reference } => {
                (reference.clone(), ReleaseSelection::Automatic)
            }
            DependencyTarget::Release { reference } => (
                ResourceRef::Source {
                    project: reference.project.clone(),
                },
                ReleaseSelection::Release {
                    reference: reference.clone(),
                },
            ),
            DependencyTarget::Artifact { reference } => (
                ResourceRef::Source {
                    project: reference.release.project.clone(),
                },
                ReleaseSelection::Artifact {
                    reference: reference.clone(),
                },
            ),
            DependencyTarget::SourceRelease { source, release_id } => {
                let value = self
                    .query(
                        source,
                        ProviderQuery::Release {
                            project_id: None,
                            release_id: release_id.clone(),
                            slug: None,
                        },
                        CachePolicy::UseFresh,
                        cancel,
                    )
                    .await?;
                let ProviderValue::Release(document) = value.value else {
                    return Err(invalid("dependency release not found"));
                };
                let reference = document.release.reference;
                (
                    ResourceRef::Source {
                        project: reference.project.clone(),
                    },
                    ReleaseSelection::Release { reference },
                )
            }
            _ => return Ok(None),
        };
        let mut selection = ResourceSelection {
            id: parent.id.clone(),
            resource,
            release,
            resource_type: parent.resource_type,
            scope: parent.scope.clone(),
            entry_id: None,
        };
        self.canonical(&mut selection, cancel).await?;
        selection.id = SelectionId::new(format!(
            "dependency:{}:{}",
            serde_json::to_string(&selection.resource)?,
            serde_json::to_string(&selection.scope)?
        ))?;
        Ok(Some(selection))
    }

    pub(crate) async fn build_operation(
        self: &Arc<Self>,
        mut draft: OperationDraft,
        target: &ResourceTarget,
        installed: &[InstalledResource],
        cancel: &CancellationToken,
    ) -> ResourceResult<OperationDraft> {
        let preferred = self.preferred_source().await?;
        draft.plan.nodes.clear();
        draft.plan.edges.clear();
        draft.plan.actions.clear();
        draft.plan.issues.clear();
        draft.plan.selection_reasons.clear();
        draft.candidates.clear();
        let mut queue: VecDeque<_> = draft.plan.selections.clone().into();
        let mut resolved: HashMap<String, SelectionId> = HashMap::new();
        let mut seen = HashSet::new();
        while let Some(mut selection) = queue.pop_front() {
            if cancel.is_cancelled() {
                return Err(ResourceError::Canceled);
            }
            if seen.len() >= 512 {
                return Err(invalid("dependency graph exceeded 512 resources"));
            }
            if !seen.insert(selection.id.clone()) {
                if let Some(previous) =
                    draft.plan.nodes.iter().find(|node| node.id == selection.id)
                {
                    let selected = previous.artifact.clone();
                    let matches = if selection_matches_artifact(
                        &selection.release,
                        selected.as_ref(),
                    ) {
                        true
                    } else {
                        let (_, expected, _) = self
                            .selected_release_for_current(
                                &selection, target, &preferred, cancel, None,
                            )
                            .await?;
                        selected.as_ref().is_some_and(|selected| {
                            identity::exact_integrity(
                                &selected.integrity,
                                &expected.integrity,
                            )
                        })
                    };
                    if !matches {
                        issue(
                            &mut draft.plan,
                            ResourceIssueCode::VersionConflict,
                            &selection.id,
                            true,
                            "multiple parents require different dependency releases",
                        );
                    }
                }
                continue;
            }
            let _metadata = match self.canonical(&mut selection, cancel).await {
                Ok(metadata) => metadata,
                Err(ResourceError::Canceled) => {
                    return Err(ResourceError::Canceled);
                }
                Err(error) => {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::SourceUnavailable,
                        &selection.id,
                        true,
                        error.to_string(),
                    );
                    continue;
                }
            };
            let previous_release = if draft.plan.intent
                == ResourceOperationIntent::Update
            {
                if let Some(artifact) = selection
                    .entry_id
                    .as_ref()
                    .and_then(|id| {
                        installed.iter().find(|entry| &entry.entry_id == id)
                    })
                    .and_then(|entry| entry.artifact.as_ref())
                {
                    Some(self.release(&artifact.release, None, cancel).await?)
                } else {
                    None
                }
            } else {
                None
            };
            let (release, artifact, basis) = match self
                .selected_release_for_current(
                    &selection,
                    target,
                    &preferred,
                    cancel,
                    previous_release.as_ref(),
                )
                .await
            {
                Ok(value) => value,
                Err(ResourceError::Canceled) => {
                    return Err(ResourceError::Canceled);
                }
                Err(error) => {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::MissingDependency,
                        &selection.id,
                        true,
                        error.to_string(),
                    );
                    continue;
                }
            };
            if artifact.reference.release.project.source
                == SourceId::curseforge()
                && self.sources().iter().any(|source| {
                    source.available && source.id == SourceId::modrinth()
                })
            {
                let matches = self
                    .identify_many(
                        &[SourceId::modrinth()],
                        &[identity::LocalArtifactIdentity {
                            integrity: artifact.integrity.clone(),
                            fingerprint: None,
                        }],
                        CachePolicy::UseFresh,
                        cancel,
                    )
                    .await;
                if let Some(matches) =
                    matches.first().filter(|matches| !matches.ambiguous)
                {
                    for other in &matches.matches {
                        if let Some(store) = self.store() {
                            let left = self
                                .project_document(
                                    &artifact.reference.release.project,
                                    None,
                                    cancel,
                                )
                                .await?;
                            let right = self
                                .project_document(
                                    &other.reference.release.project,
                                    None,
                                    cancel,
                                )
                                .await?;
                            store
                                .confirm_projects(
                                    &left.metadata,
                                    &right.metadata,
                                    &artifact,
                                    other,
                                )
                                .await?;
                        }
                    }
                }
                self.canonical(&mut selection, cancel).await?;
            }
            let resource_key = format!(
                "{}:{}",
                serde_json::to_string(&selection.resource)?,
                serde_json::to_string(&selection.scope)?
            );
            let key = selection
                .entry_id
                .as_ref()
                .map(|entry| format!("{resource_key}:entry:{}", entry.as_str()))
                .unwrap_or_else(|| resource_key.clone());
            if let Some(previous) = resolved.get(&key) {
                let node = draft
                    .plan
                    .nodes
                    .iter()
                    .find(|node| &node.id == previous)
                    .unwrap();
                if !node.artifact.as_ref().is_some_and(|other| {
                    identity::exact_integrity(
                        &other.integrity,
                        &artifact.integrity,
                    )
                }) {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::VersionConflict,
                        &selection.id,
                        true,
                        "different releases required for the same resource",
                    );
                }
                for edge in &mut draft.plan.edges {
                    if edge.child == selection.id {
                        edge.child = previous.clone();
                    }
                }
                if let Some(root) = draft
                    .plan
                    .active_roots
                    .iter_mut()
                    .find(|root| **root == selection.id)
                {
                    *root = previous.clone();
                }
                continue;
            }
            resolved.insert(key, selection.id.clone());
            resolved
                .entry(resource_key)
                .or_insert_with(|| selection.id.clone());
            let assessment =
                assess_compatibility(&release, target, selection.resource_type);
            for (dimension, value) in [
                (CompatibilityDimension::GameVersion, assessment.game_version),
                (CompatibilityDimension::Loader, assessment.loader),
            ] {
                if value == CompatibilityState::Compatible {
                    continue;
                }
                let accepted = draft.plan.exceptions.iter().any(|exception| {
                    exception.selection_id == selection.id
                        && exception.release == release.reference
                        && exception.dimensions.contains(&dimension)
                });
                if !accepted {
                    issue(
                        &mut draft.plan,
                        if value == CompatibilityState::Unknown {
                            ResourceIssueCode::CompatibilityUnknown
                        } else {
                            ResourceIssueCode::CompatibilityMismatch
                        },
                        &selection.id,
                        true,
                        format!("{:?}: {:?}", dimension, value),
                    );
                }
            }
            let candidates: Vec<_> = installed
                .iter()
                .filter(|item| {
                    item.scope == selection.scope
                        && item.resource_type == selection.resource_type
                        && match &selection.resource {
                            ResourceRef::Resource { id } => {
                                item.resource_id.as_ref() == Some(id)
                            }
                            ResourceRef::Source { project } => {
                                item.artifact.as_ref().is_some_and(|artifact| {
                                    &artifact.release.project == project
                                })
                            }
                        }
                })
                .collect();
            let current = selection.entry_id.as_ref().and_then(|entry_id| {
                installed.iter().find(|item| {
                    &item.entry_id == entry_id
                        && item.scope == selection.scope
                        && item.resource_type == selection.resource_type
                })
            });
            if let Some(current) = current {
                let same_resource = match &selection.resource {
                    ResourceRef::Resource { id } => {
                        current.resource_id.as_ref() == Some(id)
                    }
                    ResourceRef::Source { project } => {
                        current.artifact.as_ref().is_some_and(|artifact| {
                            &artifact.release.project == project
                        })
                    }
                };
                if !same_resource {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::ArtifactMismatch,
                        &selection.id,
                        true,
                        "target resource does not own the installed entry",
                    );
                }
                if draft.plan.intent == ResourceOperationIntent::Update {
                    let newer = if let Some(artifact) = &current.artifact {
                        let previous = self
                            .release(&artifact.release, None, cancel)
                            .await?;
                        release_is_newer(&release, &previous)
                    } else {
                        false
                    };
                    if !newer
                        && !identity::exact_integrity(
                            &current.integrity,
                            &artifact.integrity,
                        )
                    {
                        issue(
                            &mut draft.plan,
                            ResourceIssueCode::VersionConflict,
                            &selection.id,
                            true,
                            "target is not a newer release; use switch version for an explicit downgrade",
                        );
                    }
                }
            }
            let is_entry_update = draft.plan.intent
                != ResourceOperationIntent::Install
                && selection.entry_id.is_some();
            if is_entry_update && current.is_none() {
                issue(
                    &mut draft.plan,
                    ResourceIssueCode::ArtifactMismatch,
                    &selection.id,
                    true,
                    "the selected installed entry is missing or belongs to another scope",
                );
            }
            let satisfied = installed.iter().find(|item| {
                !item.missing
                    && item.enabled
                    && item.scope == selection.scope
                    && item.resource_type == selection.resource_type
                    && identity::exact_integrity(
                        &item.integrity,
                        &artifact.integrity,
                    )
            });
            let state = if is_entry_update {
                if satisfied.is_some() {
                    DependencyNodeState::Satisfied
                } else {
                    DependencyNodeState::Update
                }
            } else if satisfied.is_some() {
                DependencyNodeState::Satisfied
            } else if draft.plan.active_roots.contains(&selection.id) {
                DependencyNodeState::SelectedRoot
            } else {
                DependencyNodeState::Install
            };
            let path = artifact_path(
                &selection.scope,
                selection.resource_type,
                &artifact,
            );
            match path {
                Ok(_path) if is_entry_update => {
                    if let Some(current) = current {
                        let target_path = current.relative_path.clone();
                        if (draft.plan.intent
                            == ResourceOperationIntent::SwitchVersion
                            && current.artifact.as_ref()
                                != Some(&artifact.reference))
                            || !identity::exact_integrity(
                                &current.integrity,
                                &artifact.integrity,
                            )
                        {
                            draft.plan.actions.push(ResourceEntryAction {
                                entry_id: current.entry_id.clone(),
                                node_id: selection.id.clone(),
                                kind: ResourceActionKind::Replace,
                                current: Some(current.clone()),
                                target: artifact.clone(),
                                target_relative_path: target_path,
                            });
                        }
                    }
                }
                Ok(path) if satisfied.is_none() => {
                    if installed.iter().any(|item| {
                        item.relative_path.eq_ignore_ascii_case(&path)
                            && !item.missing
                    }) || draft.plan.actions.iter().any(|action| {
                        action.target_relative_path.eq_ignore_ascii_case(&path)
                    }) {
                        issue(
                            &mut draft.plan,
                            ResourceIssueCode::InvalidPath,
                            &selection.id,
                            true,
                            format!("target already exists: {path}"),
                        );
                    } else {
                        draft.plan.actions.push(ResourceEntryAction {
                            entry_id: InstalledEntryId::new(
                                uuid::Uuid::new_v4().to_string(),
                            )?,
                            node_id: selection.id.clone(),
                            kind: ResourceActionKind::Install,
                            current: None,
                            target: artifact.clone(),
                            target_relative_path: path,
                        });
                    }
                }
                Ok(_) => {}
                Err(error) => issue(
                    &mut draft.plan,
                    ResourceIssueCode::InvalidPath,
                    &selection.id,
                    true,
                    error.to_string(),
                ),
            }
            if !matches!(
                &artifact.acquisition,
                ArtifactAcquisition::PublicDownload { .. }
            ) && satisfied.is_none()
            {
                issue(
                    &mut draft.plan,
                    ResourceIssueCode::ManualDownloadRequired,
                    &selection.id,
                    true,
                    "source requires an exact manual download",
                );
            }
            draft.plan.nodes.push(ResourceDependencyNode {
                id: selection.id.clone(),
                resource: selection.resource.clone(),
                scope: selection.scope.clone(),
                state,
                artifact: Some(artifact.clone()),
                installed_entries: satisfied
                    .into_iter()
                    .map(|item| item.entry_id.clone())
                    .collect(),
                compatibility: assessment,
            });
            draft.plan.selection_reasons.push(ReleaseSelectionReason {
                node_id: selection.id.clone(),
                basis,
            });
            if let Some(root) = draft
                .plan
                .selections
                .iter_mut()
                .find(|root| root.id == selection.id)
            {
                root.resource = selection.resource.clone();
                root.release = ReleaseSelection::Artifact {
                    reference: artifact.reference.clone(),
                };
            }
            for dependency in &artifact.dependencies {
                if dependency.kind == ResourceDependencyKind::Embedded {
                    continue;
                }
                let child = match self
                    .dependency_selection(dependency, &selection, cancel)
                    .await
                {
                    Ok(Some(child)) => child,
                    Ok(None) => {
                        if dependency.kind == ResourceDependencyKind::Required {
                            issue(
                                &mut draft.plan,
                                ResourceIssueCode::MissingDependency,
                                &selection.id,
                                true,
                                "dependency requires local metadata verification",
                            );
                        }
                        continue;
                    }
                    Err(ResourceError::Canceled) => {
                        return Err(ResourceError::Canceled);
                    }
                    Err(error) => {
                        issue(
                            &mut draft.plan,
                            ResourceIssueCode::SourceUnavailable,
                            &selection.id,
                            dependency.kind == ResourceDependencyKind::Required,
                            error.to_string(),
                        );
                        continue;
                    }
                };
                let key = format!(
                    "{}:{}",
                    serde_json::to_string(&child.resource)?,
                    serde_json::to_string(&child.scope)?
                );
                let id = resolved
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| child.id.clone());
                if let Some(existing) = resolved.get(&key).and_then(|id| {
                    draft.plan.nodes.iter().find(|node| &node.id == id)
                }) {
                    if dependency.kind == ResourceDependencyKind::Required
                        && !selection_matches_artifact(
                            &child.release,
                            existing.artifact.as_ref(),
                        )
                    {
                        let existing = existing.artifact.clone();
                        let (_, expected, _) = self
                            .selected_release_for_current(
                                &child, target, &preferred, cancel, None,
                            )
                            .await?;
                        if !existing.as_ref().is_some_and(|existing| {
                            identity::exact_integrity(
                                &existing.integrity,
                                &expected.integrity,
                            )
                        }) {
                            issue(
                                &mut draft.plan,
                                ResourceIssueCode::VersionConflict,
                                &selection.id,
                                true,
                                "required dependency conflicts with an active root release or artifact",
                            );
                        }
                    }
                }
                draft.plan.edges.push(ResourceDependencyEdge {
                    parent: selection.id.clone(),
                    child: id.clone(),
                    declared_by: artifact.reference.clone(),
                    kind: dependency.kind,
                    version_constraint: dependency.version_constraint.clone(),
                });
                if dependency.version_constraint.is_some() {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::VersionConflict,
                        &selection.id,
                        dependency.kind == ResourceDependencyKind::Required,
                        "dependency version constraint needs an exact release",
                    );
                }
                if dependency.kind == ResourceDependencyKind::Required {
                    if !resolved.contains_key(&key) {
                        queue.push_back(child);
                    }
                } else if dependency.kind == ResourceDependencyKind::Optional {
                    if !draft
                        .candidates
                        .iter()
                        .any(|candidate| candidate.id == id)
                    {
                        draft
                            .candidates
                            .push(ResourceSelection { id, ..child });
                    }
                } else if installed.iter().any(|item| {
                    !item.missing
                        && item.enabled
                        && match &child.resource {
                            ResourceRef::Resource { id } => {
                                item.resource_id.as_ref() == Some(id)
                            }
                            _ => false,
                        }
                }) || resolved.contains_key(&key)
                {
                    issue(
                        &mut draft.plan,
                        ResourceIssueCode::VersionConflict,
                        &selection.id,
                        true,
                        "incompatible resource is active",
                    );
                }
            }
        }
        let mut candidates = Vec::<ResourceSelection>::new();
        let mut candidate_ids = HashMap::<String, SelectionId>::new();
        let mut installed = installed.to_vec();
        if let Some(store) = self.store() {
            for node in &mut draft.plan.nodes {
                if let ResourceRef::Resource { id } = &mut node.resource {
                    if let Some(current) = store.resolve(id).await? {
                        *id = current;
                    }
                }
            }
            for item in &mut installed {
                if let Some(id) = &item.resource_id {
                    item.resource_id = store.resolve(id).await?;
                }
            }
        }
        for mut candidate in draft.candidates.clone() {
            if let Some(store) = self.store() {
                let current = match &candidate.resource {
                    ResourceRef::Source { project } => {
                        store.lookup(project).await?
                    }
                    ResourceRef::Resource { id } => store.resolve(id).await?,
                };
                if let Some(id) = current {
                    candidate.resource = ResourceRef::Resource { id };
                }
            }
            let key = format!(
                "{}:{}",
                serde_json::to_string(&candidate.resource)?,
                serde_json::to_string(&candidate.scope)?
            );
            let active = draft.plan.nodes.iter().find(|node| {
                node.resource == candidate.resource
                    && node.scope == candidate.scope
            });
            let id = active
                .map(|node| node.id.clone())
                .or_else(|| candidate_ids.get(&key).cloned())
                .unwrap_or_else(|| candidate.id.clone());
            for edge in &mut draft.plan.edges {
                if edge.child == candidate.id {
                    edge.child = id.clone();
                }
            }
            if active.is_some() || candidate_ids.contains_key(&key) {
                continue;
            }
            candidate_ids.insert(key, id.clone());
            candidate.id = id;
            let satisfied: Vec<_> = installed
                .iter()
                .filter(|item| {
                    !item.missing
                        && item.enabled
                        && item.scope == candidate.scope
                        && item.resource_type == candidate.resource_type
                        && match &candidate.resource {
                            ResourceRef::Resource { id } => {
                                item.resource_id.as_ref() == Some(id)
                            }
                            ResourceRef::Source { project } => {
                                item.artifact.as_ref().is_some_and(|artifact| {
                                    &artifact.release.project == project
                                })
                            }
                        }
                })
                .map(|item| item.entry_id.clone())
                .collect();
            let node_resource = if satisfied.is_empty() {
                candidate.resource.clone()
            } else {
                let (_, projects) =
                    self.source_projects(&candidate.resource).await?;
                ResourceRef::Source {
                    project: projects[0].clone(),
                }
            };
            draft.plan.nodes.push(ResourceDependencyNode {
                id: candidate.id.clone(),
                resource: node_resource,
                scope: candidate.scope.clone(),
                state: if satisfied.is_empty() {
                    DependencyNodeState::OptionalCandidate
                } else {
                    DependencyNodeState::Satisfied
                },
                artifact: None,
                installed_entries: satisfied.clone(),
                compatibility: CompatibilityAssessment {
                    game_version: CompatibilityState::Unknown,
                    loader: CompatibilityState::Unknown,
                    reasons: vec![],
                },
            });
            if satisfied.is_empty() {
                candidates.push(candidate);
            }
        }
        draft.candidates = candidates;
        Ok(draft)
    }
}

pub(crate) fn update_channels(
    channel: crate::state::ReleaseChannel,
) -> Vec<ReleaseChannel> {
    match channel {
        crate::state::ReleaseChannel::Release => vec![ReleaseChannel::Release],
        crate::state::ReleaseChannel::Beta => {
            vec![ReleaseChannel::Release, ReleaseChannel::Beta]
        }
        crate::state::ReleaseChannel::Alpha => vec![
            ReleaseChannel::Release,
            ReleaseChannel::Beta,
            ReleaseChannel::Alpha,
        ],
    }
}

pub async fn prepare(
    instance_id: String,
    intent: ResourceOperationIntent,
    selections: Vec<ResourceSelection>,
    install_dependencies: bool,
    cancel: &CancellationToken,
) -> ResourceResult<ResourceOperationPlan> {
    match intent {
        ResourceOperationIntent::Install
            if selections
                .iter()
                .any(|selection| selection.entry_id.is_some()) =>
        {
            return Err(invalid(
                "install roots cannot target an existing entry",
            ));
        }
        ResourceOperationIntent::Update
        | ResourceOperationIntent::SwitchVersion
            if selections
                .iter()
                .all(|selection| selection.entry_id.is_none()) =>
        {
            return Err(invalid("updates must identify an installed entry"));
        }
        _ => {}
    }
    if selections.is_empty()
        || selections.len() > 100
        || selections
            .iter()
            .map(|selection| &selection.id)
            .collect::<HashSet<_>>()
            .len()
            != selections.len()
    {
        return Err(invalid("select between one and 100 distinct roots"));
    }
    let state = crate::State::get().await?;
    let instance = crate::state::get_instance(&instance_id, &state.pool)
        .await?
        .ok_or_else(|| invalid("unknown instance"))?;
    let revision = instance_revision(&state.pool, &instance_id).await?;
    let roots = selections
        .iter()
        .map(|selection| selection.id.clone())
        .collect();
    let draft = OperationDraft {
        install_dependencies,
        candidates: vec![],
        local_files: HashMap::new(),
        restore_history: None,
        plan: ResourceOperationPlan {
            schema_version: ResourceContractVersion,
            id: PlanId::new(uuid::Uuid::new_v4().to_string())?,
            revision: 1,
            instance_id,
            instance_revision: revision,
            intent,
            selections,
            active_roots: roots,
            nodes: vec![],
            edges: vec![],
            actions: vec![],
            issues: vec![],
            exceptions: vec![],
            selection_reasons: vec![],
        },
    };
    let target = ResourceTarget {
        game_versions: vec![instance.applied_content_set.game_version],
        loaders: vec![instance.applied_content_set.loader.as_str().into()],
        channels: update_channels(instance.instance.update_channel),
    };
    let installed = if draft.plan.intent == ResourceOperationIntent::Install {
        installed(&state.pool, &draft.plan.instance_id).await?
    } else {
        super::installed::hydrate(&state, &draft.plan.instance_id, cancel)
            .await?
    };
    let draft = state
        .resources()
        .await?
        .build_operation(draft, &target, &installed, cancel)
        .await?;
    if cancel.is_cancelled() {
        return Err(ResourceError::Canceled);
    }
    if instance_revision(&state.pool, &draft.plan.instance_id).await?
        != revision
    {
        return Err(invalid("instance changed while preparing the plan"));
    }
    sqlx::query("INSERT INTO resource_operation_plans(id, instance_id, revision, payload) VALUES (?, ?, ?, ?)").bind(draft.plan.id.as_str()).bind(&draft.plan.instance_id).bind(draft.plan.revision).bind(serde_json::to_string(&draft)?).execute(&state.pool).await.map_err(db)?;
    Ok(draft.plan)
}

pub(crate) async fn load(
    pool: &SqlitePool,
    id: &PlanId,
    revision: u32,
) -> ResourceResult<OperationDraft> {
    let payload: Option<String> = sqlx::query_scalar("SELECT payload FROM resource_operation_plans WHERE id = ? AND revision = ? AND job_id IS NULL AND queued = 0").bind(id.as_str()).bind(revision).fetch_optional(pool).await.map_err(db)?;
    serde_json::from_str(
        &payload
            .ok_or_else(|| invalid("plan was revised or already queued"))?,
    )
    .map_err(Into::into)
}

pub async fn revise(
    id: PlanId,
    revision: u32,
    decisions: Vec<ResourcePlanDecision>,
    cancel: &CancellationToken,
) -> ResourceResult<ResourceOperationPlan> {
    let state = crate::State::get().await?;
    let mut draft = load(&state.pool, &id, revision).await?;
    if instance_revision(&state.pool, &draft.plan.instance_id).await?
        != draft.plan.instance_revision
    {
        return Err(invalid("instance changed; prepare a new plan"));
    }
    let added_optional = decisions.iter().any(|decision| {
        matches!(decision, ResourcePlanDecision::AddOptional { .. })
    });
    let previous_issues = draft.plan.issues.clone();
    for decision in decisions {
        match decision {
            ResourcePlanDecision::AddOptional { node_id } => {
                let candidate = draft
                    .candidates
                    .iter()
                    .find(|selection| selection.id == node_id)
                    .ok_or_else(|| invalid("unknown dependency candidate"))?
                    .clone();
                if !draft
                    .plan
                    .selections
                    .iter()
                    .any(|selection| selection.id == node_id)
                {
                    draft.plan.selections.push(candidate);
                    draft.plan.active_roots.push(node_id);
                }
            }
            ResourcePlanDecision::RemoveOptional { node_id }
            | ResourcePlanDecision::RemoveRoot {
                selection_id: node_id,
            } => {
                draft
                    .plan
                    .selections
                    .retain(|selection| selection.id != node_id);
                draft.plan.active_roots.retain(|root| root != &node_id);
            }
            ResourcePlanDecision::AcceptCompatibility { exception } => {
                if !draft.plan.nodes.iter().any(|node| {
                    node.id == exception.selection_id
                        && node.artifact.as_ref().is_some_and(|artifact| {
                            artifact.reference.release == exception.release
                        })
                }) {
                    return Err(invalid(
                        "compatibility exception does not match this frozen release",
                    ));
                }
                draft.plan.exceptions.push(exception);
            }
            ResourcePlanDecision::SelectRelease { node_id, selection } => {
                draft
                    .plan
                    .selections
                    .iter_mut()
                    .find(|root| root.id == node_id)
                    .ok_or_else(|| {
                        invalid("only active roots can change release")
                    })?
                    .release = selection;
            }
        }
    }
    draft.plan.revision = revision
        .checked_add(1)
        .ok_or_else(|| invalid("plan revision overflow"))?;
    let instance =
        crate::state::get_instance(&draft.plan.instance_id, &state.pool)
            .await?
            .ok_or_else(|| invalid("unknown instance"))?;
    let target = ResourceTarget {
        game_versions: vec![instance.applied_content_set.game_version],
        loaders: vec![instance.applied_content_set.loader.as_str().into()],
        channels: update_channels(instance.instance.update_channel),
    };
    let installed = if draft.plan.intent == ResourceOperationIntent::Install {
        installed(&state.pool, &draft.plan.instance_id).await?
    } else {
        super::installed::hydrate(&state, &draft.plan.instance_id, cancel)
            .await?
    };
    let draft = state
        .resources()
        .await?
        .build_operation(draft, &target, &installed, cancel)
        .await?;
    if added_optional
        && draft.plan.issues.iter().any(|issue| {
            issue.blocking
                && matches!(
                    issue.code,
                    ResourceIssueCode::SourceUnavailable
                        | ResourceIssueCode::MissingDependency
                )
                && !previous_issues.contains(issue)
        })
    {
        let reasons = draft
            .plan
            .issues
            .iter()
            .filter(|issue| issue.blocking && !previous_issues.contains(issue))
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(invalid(format!(
            "unable to resolve the added dependency; your previous selection is retained: {reasons}",
        )));
    }
    if cancel.is_cancelled() {
        return Err(ResourceError::Canceled);
    }
    if instance_revision(&state.pool, &draft.plan.instance_id).await?
        != draft.plan.instance_revision
    {
        return Err(invalid("instance changed while revising the plan"));
    }
    let result = sqlx::query("UPDATE resource_operation_plans SET revision = ?, payload = ? WHERE id = ? AND revision = ? AND job_id IS NULL AND queued = 0").bind(draft.plan.revision).bind(serde_json::to_string(&draft)?).bind(id.as_str()).bind(revision).execute(&state.pool).await.map_err(db)?;
    if result.rows_affected() != 1 {
        return Err(invalid("plan changed concurrently"));
    }
    Ok(draft.plan)
}

pub async fn queue(
    id: PlanId,
    revision: u32,
) -> ResourceResult<crate::install::InstallJobSnapshot> {
    crate::install::runner::queue_resource_plan(id, revision)
        .await
        .map_err(|error| ResourceError::InvalidRequest(error.to_string()))
}

pub(crate) async fn execute_frozen_plan(
    instance_id: &str,
    plan: &ResourceOperationPlan,
    reporter: Option<crate::install::InstallProgressReporter>,
    state: &crate::State,
) -> crate::Result<()> {
    super::execution::execute(instance_id, plan, reporter, state).await
}

pub(crate) async fn recover_frozen_plan(
    plan: &ResourceOperationPlan,
    state: &crate::State,
) -> crate::Result<()> {
    super::execution::recover_plan(plan, state).await
}
