//! CurseForge install dependencies.

use super::*;
use crate::State;
use crate::api::curseforge::catalog::get_project;
use crate::api::curseforge::constants::{
    CURSEFORGE_LOADER_QUILT, MAX_DEPENDENCY_DEPTH,
};
use crate::api::curseforge::dto::{
    CurseForgeFailedDownload, CurseForgeFile, CurseForgeInstallRequest,
    CurseForgeInstallResult, CurseForgeProject, CurseForgeSkippedDependency,
};
use crate::api::curseforge::mapping::recognized_project_type;
use crate::api::curseforge::selection::{
    resolve_modrinth_fallback_plan, select_dependency_file,
};
use crate::state::ProjectType;
use crate::state::instances::DependencyRelation;
use content_management::{DependencyPolicy, DependencyType};
use std::collections::{HashMap, HashSet};

pub(crate) fn dependency_policy(
    operation: crate::state::instances::ManualDownloadOperationKind,
) -> DependencyPolicy {
    if operation
        == crate::state::instances::ManualDownloadOperationKind::ContentUpdate
    {
        DependencyPolicy::RequiredOnly
    } else {
        DependencyPolicy::IncludeOptional
    }
}

pub(crate) fn should_resolve_dependency(
    relation: DependencyRelation,
    operation: crate::state::instances::ManualDownloadOperationKind,
) -> bool {
    let dependency_type = match relation {
        DependencyRelation::Required => DependencyType::Required,
        DependencyRelation::Optional | DependencyRelation::Include => {
            DependencyType::Optional
        }
        _ => return false,
    };
    dependency_policy(operation).allows(dependency_type)
}

pub(crate) async fn enqueue_curseforge_dependencies(
    pending_file: &PendingCurseForgeFile,
    file: &CurseForgeFile,
    project_id: u32,
    file_id: u32,
    item_type: ProjectType,
    request: &CurseForgeInstallRequest,
    state: &State,
    installed_project_ids: &[String],
    projects: &mut HashMap<u32, CurseForgeProject>,
    pending: &mut Vec<PendingCurseForgeFile>,
    result: &mut CurseForgeInstallResult,
    modrinth_fallbacks: &mut Vec<ModrinthFallbackPlan>,
    fallback_parents: &mut HashSet<(u32, u32)>,
) -> crate::Result<()> {
    if !request.install_dependencies {
        return Ok(());
    }
    if pending_file.depth >= MAX_DEPENDENCY_DEPTH {
        result
            .skipped_dependencies
            .push(CurseForgeSkippedDependency {
                project_id,
                file_id: Some(file_id),
                reason: "dependency_depth_exceeded".to_string(),
            });
        return Ok(());
    }

    for dependency_ref in &file.dependencies {
        let relation = dependency_ref.relation();
        match relation {
            DependencyRelation::Embedded | DependencyRelation::Tool => {
                result
                    .skipped_dependencies
                    .push(CurseForgeSkippedDependency {
                        project_id: dependency_ref.mod_id,
                        file_id: None,
                        reason: if relation == DependencyRelation::Embedded {
                            "embedded"
                        } else {
                            "tool"
                        }
                        .to_string(),
                    });
            }
            DependencyRelation::Incompatible => {
                result.incompatible_dependencies.push(dependency_ref.mod_id)
            }
            DependencyRelation::Optional
            | DependencyRelation::Include
            | DependencyRelation::Required => {
                if !should_resolve_dependency(
                    relation,
                    request.manual_operation_kind,
                ) {
                    continue;
                }
                let dependency_project_id = dependency_ref.target_project_id(
                    request.mod_loader_type == Some(CURSEFORGE_LOADER_QUILT),
                );
                if request
                    .excluded_dependency_project_ids
                    .contains(&dependency_project_id)
                {
                    result.skipped_dependencies.push(
                        CurseForgeSkippedDependency {
                            project_id: dependency_ref.mod_id,
                            file_id: None,
                            reason: "excluded_by_user".to_string(),
                        },
                    );
                    continue;
                }
                if installed_project_ids
                    .contains(&format!("curseforge:{dependency_project_id}"))
                    && !request
                        .force_dependency_project_ids
                        .contains(&dependency_project_id)
                {
                    result.skipped_dependencies.push(
                        CurseForgeSkippedDependency {
                            project_id: dependency_ref.mod_id,
                            file_id: None,
                            reason: "already_installed".to_string(),
                        },
                    );
                    continue;
                }
                let dependency_project = match projects
                    .get(&dependency_project_id)
                {
                    Some(project) => project.clone(),
                    None => {
                        let project =
                            get_project(dependency_project_id).await?;
                        projects.insert(dependency_project_id, project.clone());
                        project
                    }
                };
                let Some(dependency_type) =
                    recognized_project_type(dependency_project.class_id)
                else {
                    result.failed_downloads.push(CurseForgeFailedDownload {
                        project_id: dependency_project_id,
                        file_id: 0,
                        file_name: dependency_project.name.clone(),
                        reason: "The dependency project type is not supported"
                            .to_string(),
                    });
                    continue;
                };
                if let Some(selected) = select_dependency_file(
                    dependency_project_id,
                    request.game_version.clone(),
                    request.mod_loader_type,
                )
                .await?
                {
                    pending.push(PendingCurseForgeFile {
                        project_id: dependency_project_id,
                        file_id: selected.file.id,
                        item_type: dependency_type,
                        dependency: true,
                        parent_project_id: Some(project_id),
                        parent_file_id: Some(file_id),
                        dependency_kind: Some(if relation.is_required() {
                            crate::state::instances::ContentDependencyKind::Required
                        } else {
                            crate::state::instances::ContentDependencyKind::Include
                        }),
                        depth: pending_file.depth + 1,
                    });
                } else if fallback_parents.insert((project_id, file_id)) {
                    match resolve_modrinth_fallback_plan(
                        file, item_type, request, state,
                    )
                    .await
                    {
                        Ok(Some(plan)) if !plan.dependencies.is_empty() => {
                            modrinth_fallbacks.push(ModrinthFallbackPlan {
                                parent_project_id: project_id,
                                parent_file_id: file_id,
                                plan,
                            });
                        }
                        Ok(Some(_) | None) => {
                            result.skipped_dependencies.push(
                                CurseForgeSkippedDependency {
                                    project_id: dependency_project_id,
                                    file_id: None,
                                    reason: "no_compatible_version".to_string(),
                                },
                            );
                        }
                        Err(error) => {
                            tracing::warn!(
                                project_id = dependency_project_id,
                                parent_project_id = project_id,
                                parent_file_id = file_id,
                                "Modrinth SHA-1 dependency fallback failed: {error}"
                            );
                            result.skipped_dependencies.push(
                                CurseForgeSkippedDependency {
                                    project_id: dependency_project_id,
                                    file_id: None,
                                    reason: "modrinth_lookup_failed"
                                        .to_string(),
                                },
                            );
                        }
                    }
                } else {
                    result.skipped_dependencies.push(
                        CurseForgeSkippedDependency {
                            project_id: dependency_project_id,
                            file_id: None,
                            reason: "no_compatible_version".to_string(),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Completes the common post-processing for a CurseForge installation.
///
/// Keeping fallback installation, result normalization, and dependency-edge
/// persistence together makes the main dependency walk easier to reason about
/// and gives preview/resolution-plan flows a single place to reuse later.
pub(crate) async fn finalize_install_result(
    instance_id: &str,
    modrinth_fallbacks: &[ModrinthFallbackPlan],
    result: &mut CurseForgeInstallResult,
    edge_candidates: &[CurseForgeDependencyEdgeCandidate],
    state: &State,
) -> crate::Result<()> {
    install_modrinth_fallbacks(instance_id, modrinth_fallbacks, result, state)
        .await?;

    result.optional_dependencies.sort_unstable();
    result.optional_dependencies.dedup();
    result.incompatible_dependencies.sort_unstable();
    result.incompatible_dependencies.dedup();
    result.skipped_dependencies.sort_unstable();
    result.skipped_dependencies.dedup();

    persist_curseforge_dependency_edges(instance_id, edge_candidates, state)
        .await?;
    Ok(())
}
