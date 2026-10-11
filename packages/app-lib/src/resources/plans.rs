use super::{ipc::ResourceTarget, providers::*, *};
use crate::state::{
    ContentProviderRef, ContentSourceKind, CurseForgeFileId,
    CurseForgeProjectId, ModrinthProjectId, ModrinthVersionId, ProjectType,
};
use crate::util::fetch::{self, DownloadRequest, Integrity, ResourceClass};
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
    let value: Option<i64> = sqlx::query_scalar("SELECT revision FROM instance_content_sets WHERE instance_id = ? AND status = 'applied'").bind(instance_id).fetch_optional(pool).await.map_err(db)?;
    value
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| invalid("instance has no applied content set"))
}

pub(crate) async fn installed(
    pool: &SqlitePool,
    instance_id: &str,
) -> ResourceResult<Vec<InstalledResource>> {
    let rows = sqlx::query("SELECT entry.id, entry.project_type, entry.ownership_kind, file.relative_path, file.enabled, file.missing, file.sha1, file.size, exact.artifact, exact.scope FROM instance_content_entries entry JOIN instance_content_sets content ON content.id = entry.content_set_id JOIN instance_files file ON file.id = entry.file_id LEFT JOIN resource_installed_artifacts exact ON exact.entry_id = entry.id WHERE entry.instance_id = ? AND content.status = 'applied'").bind(instance_id).fetch_all(pool).await.map_err(db)?;
    let mut result = vec![];
    for row in rows {
        let entry_id: String = row.get("id");
        let kind: String = row.get("project_type");
        let resource_type = match kind.as_str() {
            "mod" => ResourceType::Mod,
            "plugin" => ResourceType::Plugin,
            "shader" => ResourceType::Shader,
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
        let resource_id = links.first().and_then(|row| {
            ResourceId::new(row.get::<String, _>("resource_id")).ok()
        });
        let artifact = exact.as_ref().map(|file| file.reference.clone());
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
                    pack_member_id: "existing-pack-member".into(),
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

    async fn selected_release(
        self: &Arc<Self>,
        selection: &ResourceSelection,
        target: &ResourceTarget,
        preferred: &SourceId,
        cancel: &CancellationToken,
    ) -> ResourceResult<(
        ResourceRelease,
        ResourceArtifact,
        ReleaseSelectionBasis,
    )> {
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
                for project in projects {
                    let mut cursor = None;
                    let mut seen = HashSet::new();
                    loop {
                        let slug = if let Some(store) = self.store() {
                            store.project_alias(&project).await?
                        } else {
                            None
                        };
                        let value = self
                            .query(
                                &project.source,
                                ProviderQuery::Releases {
                                    project_id: project.project_id.clone(),
                                    slug,
                                    target: target.clone(),
                                    cursor: cursor.clone(),
                                    page_size: 50,
                                },
                                CachePolicy::UseFresh,
                                cancel,
                            )
                            .await?;
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
                                    && (target.channels.is_empty()
                                        || target
                                            .channels
                                            .contains(&release.channel))
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
                select_automatic_release(releases, preferred)
                    .ok_or_else(|| invalid("no compatible release found"))?
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
            let key = format!(
                "{}:{}",
                serde_json::to_string(&selection.resource)?,
                serde_json::to_string(&selection.scope)?
            );
            let (release, artifact, basis) = match self
                .selected_release(&selection, target, &preferred, cancel)
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
            let satisfied = candidates
                .iter()
                .find(|item| {
                    !item.missing
                        && item.enabled
                        && identity::exact_integrity(
                            &item.integrity,
                            &artifact.integrity,
                        )
                })
                .copied();
            let state = if satisfied.is_some() {
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
                if dependency.kind == ResourceDependencyKind::Required
                    && draft.install_dependencies
                {
                    if !resolved.contains_key(&key) {
                        queue.push_back(child);
                    }
                } else if dependency.kind
                    != ResourceDependencyKind::Incompatible
                {
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
        for candidate in draft.candidates.clone() {
            if draft.plan.nodes.iter().any(|node| node.id == candidate.id) {
                continue;
            }
            let _metadata = self
                .get(&candidate.resource, None, None, cancel)
                .await
                .ok()
                .and_then(|resource| resource.projects.into_iter().next());
            draft.plan.nodes.push(ResourceDependencyNode {
                id: candidate.id.clone(),
                resource: candidate.resource,
                scope: candidate.scope,
                state: DependencyNodeState::OptionalCandidate,
                artifact: None,
                installed_entries: vec![],
                compatibility: CompatibilityAssessment {
                    game_version: CompatibilityState::Unknown,
                    loader: CompatibilityState::Unknown,
                    reasons: vec![],
                },
            });
            if draft.plan.edges.iter().any(|edge| {
                edge.child == candidate.id
                    && edge.kind == ResourceDependencyKind::Required
            }) {
                issue(
                    &mut draft.plan,
                    ResourceIssueCode::MissingDependency,
                    &candidate.id,
                    true,
                    "dependency auto-install is disabled; explicitly add the required resource",
                );
            }
        }
        Ok(draft)
    }
}

pub async fn prepare(
    instance_id: String,
    intent: ResourceOperationIntent,
    selections: Vec<ResourceSelection>,
    install_dependencies: bool,
    cancel: &CancellationToken,
) -> ResourceResult<ResourceOperationPlan> {
    if intent != ResourceOperationIntent::Install
        || selections
            .iter()
            .any(|selection| selection.entry_id.is_some())
    {
        return Err(ResourceError::Unsupported(
            "entry updates use the content change workflow until phase six"
                .into(),
        ));
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
        channels: vec![],
    };
    let installed = installed(&state.pool, &draft.plan.instance_id).await?;
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
        channels: vec![],
    };
    let installed = installed(&state.pool, &draft.plan.instance_id).await?;
    let draft = state
        .resources()
        .await?
        .build_operation(draft, &target, &installed, cancel)
        .await?;
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
) -> ResourceResult<ResourceOperationPlan> {
    let state = crate::State::get().await?;
    let draft = load(&state.pool, &id, revision).await?;
    if draft.plan.issues.iter().any(|issue| issue.blocking) {
        return Err(invalid("resolve blocking plan issues before queueing"));
    }
    let claimed = sqlx::query("UPDATE resource_operation_plans SET queued = 1 WHERE id = ? AND revision = ? AND job_id IS NULL AND queued = 0").bind(id.as_str()).bind(revision).execute(&state.pool).await.map_err(db)?;
    if claimed.rows_affected() != 1 {
        return Err(invalid("plan changed concurrently or is already queued"));
    }
    let item = crate::install::InstallContentBatchItem::FrozenResource {
        plan: draft.plan.clone(),
    };
    let job = match crate::install::runner::install_content_batch(
        draft.plan.instance_id.clone(),
        vec![item],
        "Install resources".into(),
        None,
    )
    .await
    {
        Ok(job) => job,
        Err(error) => {
            let _ = sqlx::query("UPDATE resource_operation_plans SET queued = 0 WHERE id = ? AND revision = ? AND job_id IS NULL")
                .bind(id.as_str()).bind(revision).execute(&state.pool).await;
            return Err(ResourceError::Database(error.to_string()));
        }
    };
    let changed = sqlx::query("UPDATE resource_operation_plans SET job_id = ? WHERE id = ? AND revision = ? AND job_id IS NULL AND queued = 1")
        .bind(job.job_id.to_string()).bind(id.as_str()).bind(revision).execute(&state.pool).await.map_err(db)?;
    if changed.rows_affected() != 1 {
        return Err(invalid("plan changed concurrently or is already queued"));
    }
    Ok(draft.plan)
}

pub(crate) async fn execute_frozen_plan(
    instance_id: &str,
    plan: &ResourceOperationPlan,
    reporter: Option<crate::install::InstallProgressReporter>,
    state: &crate::State,
) -> crate::Result<()> {
    if plan.instance_id != instance_id
        || plan.issues.iter().any(|issue| issue.blocking)
    {
        return Err(crate::ErrorKind::InputError(
            "resource plan is not executable".into(),
        )
        .into());
    }
    let current = instance_revision(&state.pool, instance_id)
        .await
        .map_err(|error| crate::ErrorKind::InputError(error.to_string()))?;
    if current != plan.instance_revision {
        return Err(crate::ErrorKind::InputError(
            "resource plan is stale".into(),
        )
        .into());
    }
    let temp = tempfile::tempdir()?;
    let total = plan.actions.len() as u64;
    for (index, action) in plan.actions.iter().enumerate() {
        let url = match &action.target.acquisition {
            ArtifactAcquisition::PublicDownload { urls } => {
                urls.first().cloned().ok_or_else(|| {
                    crate::ErrorKind::InputError(
                        "artifact has no public URL".into(),
                    )
                })?
            }
            _ => {
                return Err(crate::ErrorKind::InputError(
                    "artifact requires a manual download".into(),
                )
                .into());
            }
        };
        let file_name = action.target.file_name.clone().ok_or_else(|| {
            crate::ErrorKind::InputError("artifact has no filename".into())
        })?;
        let path = temp.path().join(&file_name);
        let mut integrity = Integrity::default();
        integrity.size = action.target.integrity.size;
        for hash in &action.target.integrity.hashes {
            match hash.algorithm {
                HashAlgorithm::Sha1 => {
                    integrity.sha1 = Some(hash.value.clone())
                }
                HashAlgorithm::Sha256 => {
                    integrity.sha256 = Some(hash.value.clone())
                }
                HashAlgorithm::Sha512 => {
                    integrity.sha512 = Some(hash.value.clone())
                }
                HashAlgorithm::Md5 => integrity.md5 = Some(hash.value.clone()),
            }
        }
        let mut request = DownloadRequest::new(url, ResourceClass::Other)
            .with_integrity(integrity);
        if let Some(reporter) = reporter.clone() {
            request = request.with_install_tracking(
                reporter,
                action.target_relative_path.clone(),
                file_name.clone(),
            );
        }
        fetch::download_to_path(
            request,
            &path,
            &state.download_semaphore,
            &state.pool,
            None,
        )
        .await?;
        let provider = &action.target.reference.release.project;
        let provider_ref = match provider.source.as_str() {
            "modrinth" => ContentProviderRef::Modrinth {
                project_id: ModrinthProjectId::new(
                    provider.project_id.as_str(),
                )?,
                version_id: Some(ModrinthVersionId::new(
                    action.target.reference.release.release_id.as_str(),
                )?),
            },
            "curseforge" => ContentProviderRef::CurseForge {
                project_id: CurseForgeProjectId::new(
                    provider.project_id.as_str().parse().map_err(|_| {
                        crate::ErrorKind::InputError(
                            "invalid CurseForge project ID".to_string(),
                        )
                    })?,
                )?,
                file_id: Some(CurseForgeFileId::new(
                    action
                        .target
                        .reference
                        .release
                        .release_id
                        .as_str()
                        .parse()
                        .map_err(|_| {
                            crate::ErrorKind::InputError(
                                "invalid CurseForge file ID".to_string(),
                            )
                        })?,
                )?),
            },
            _ => {
                return Err(crate::ErrorKind::InputError(
                    "this source cannot yet materialize a content entry".into(),
                )
                .into());
            }
        };
        let project_type = match plan
            .selections
            .iter()
            .find(|selection| selection.id == action.node_id)
            .map(|selection| selection.resource_type)
        {
            Some(ResourceType::Mod | ResourceType::Plugin) => ProjectType::Mod,
            Some(ResourceType::DataPack) => ProjectType::DataPack,
            Some(ResourceType::ResourcePack) => ProjectType::ResourcePack,
            Some(ResourceType::Shader) => ProjectType::ShaderPack,
            _ => {
                return Err(crate::ErrorKind::InputError(
                    "unsupported resource type".into(),
                )
                .into());
            }
        };
        let bytes = tokio::fs::read(&path).await?;
        crate::state::instances::commands::add_project_bytes_from_provider(
            instance_id,
            &file_name,
            bytes::Bytes::from(bytes),
            None,
            project_type,
            ContentSourceKind::Local,
            &provider_ref,
            state,
        )
        .await?;
        if let Some(reporter) = reporter.clone() {
            reporter
                .update(
                    crate::install::InstallPhaseId::DownloadingContent,
                    Some(crate::install::InstallProgress {
                        current: (index + 1) as u64,
                        total,
                        secondary: None,
                    }),
                    crate::install::InstallPhaseDetails::Empty,
                )
                .await?;
        }
    }
    let state_json = serde_json::to_string(plan)?;
    sqlx::query("UPDATE resource_operation_plans SET completed = 1, payload = ? WHERE id = ? AND revision = ?")
        .bind(state_json).bind(plan.id.as_str()).bind(plan.revision).execute(&state.pool).await?;
    Ok(())
}
