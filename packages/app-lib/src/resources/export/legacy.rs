//! Phase-eight MR writer boundary; consumes fresh evidence without new provider requests.
use super::refresh::{ExportDraft, freeze};
use crate::pack::install_from::{
    EnvType, PackDependency, PackFile, PackFileHash, PackFormat,
};
use crate::resources::*;
use crate::state::SideType;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

fn invalid(message: &str) -> ResourceError {
    ResourceError::InvalidRequest(message.into())
}

pub fn validate_choices(
    draft: &ExportDraft,
    revision: u32,
    choices: &[ExportEmbeddedFailureChoice],
) -> ResourceResult<()> {
    if revision != draft.selection_revision {
        return Err(invalid(
            "export selection changed; refresh before exporting",
        ));
    }
    for path in &draft.selected {
        let result = draft
            .results
            .get(path)
            .ok_or_else(|| invalid("export analysis is incomplete"))?;
        if result.local_error.is_some() {
            return Err(invalid(
                "an unreadable local file cannot be embedded; retry or remove it",
            ));
        }
        if result.snapshot.integrity.hashes.len() < 3 {
            return Err(invalid("export file has no frozen integrity"));
        }
        if result.snapshot.identifiable
            && [SourceId::modrinth(), SourceId::curseforge()].iter().any(
                |source| {
                    !result.resolutions.iter().any(|r| match r {
                        ExportSourceResolution::Matched {
                            source: s, ..
                        }
                        | ExportSourceResolution::NoMatch { source: s }
                        | ExportSourceResolution::Failed {
                            source: s, ..
                        } => s == source,
                    })
                },
            )
        {
            return Err(invalid("export analysis is incomplete"));
        }
        for resolution in &result.resolutions {
            if let ExportSourceResolution::Failed { source, .. } = resolution {
                if !choices.iter().any(|choice| {
                    choice.snapshot == result.snapshot
                        && &choice.source == source
                }) {
                    return Err(invalid(
                        "failed source lookup requires retry or an explicit embedding choice",
                    ));
                }
            }
        }
    }
    if choices.iter().any(|choice| !draft.selected.contains(&choice.snapshot.relative_path) || !draft.results.get(&choice.snapshot.relative_path).is_some_and(|file| file.snapshot == choice.snapshot && file.resolutions.iter().any(|r| matches!(r, ExportSourceResolution::Failed { source, .. } if source == &choice.source)))) { return Err(invalid("embedding choice no longer matches the failed file")); }
    Ok(())
}

pub fn mr_pack(
    draft: &ExportDraft,
    info: ExportPackageInfo,
    choices: &[ExportEmbeddedFailureChoice],
) -> ResourceResult<PackFormat> {
    let mut dependencies = HashMap::new();
    dependencies.insert(
        PackDependency::Minecraft,
        draft.scan.requirements.game_version.clone(),
    );
    if let Some(loader) = &draft.scan.requirements.loader {
        let key = match loader.as_str() {
            "forge" => PackDependency::Forge,
            "neoforge" => PackDependency::NeoForge,
            "fabric" => PackDependency::FabricLoader,
            "quilt" => PackDependency::QuiltLoader,
            "cleanroom" => PackDependency::Cleanroom,
            _ => {
                return Err(invalid(
                    "this loader cannot be expressed by the MR format",
                ));
            }
        };
        dependencies.insert(
            key,
            draft
                .scan
                .requirements
                .loader_version
                .clone()
                .ok_or_else(|| invalid("loader version missing"))?,
        );
    }
    let mut files = vec![];
    for path in &draft.selected {
        let result = &draft.results[path];
        if path.ends_with(".disabled")
            || path.ends_with(".old")
            || choices
                .iter()
                .any(|choice| choice.snapshot == result.snapshot)
        {
            continue;
        }
        let artifact = result.resolutions.iter().find_map(|resolution| if let ExportSourceResolution::Matched { source, artifacts } = resolution { if source == &SourceId::modrinth() { artifacts.iter().find(|a| matches!(&a.acquisition, ArtifactAcquisition::PublicDownload { urls } if !urls.is_empty())) } else { None } } else { None });
        let Some(artifact) = artifact else {
            continue;
        };
        let ArtifactAcquisition::PublicDownload { urls } =
            &artifact.acquisition
        else {
            continue;
        };
        let project = draft.projects.get(&artifact.reference.release.project);
        let side = |key: &str| {
            project
                .and_then(|p| p.protocol.as_ref())
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_str())
                .and_then(|v| match v {
                    "required" => Some(SideType::Required),
                    "optional" => Some(SideType::Optional),
                    "unsupported" => Some(SideType::Unsupported),
                    _ => None,
                })
        };
        let environment = match (side("client_side"), side("server_side")) {
            (Some(client), Some(server)) => Some(HashMap::from([
                (EnvType::Client, client),
                (EnvType::Server, server),
            ])),
            _ => None,
        };
        files.push(PackFile {
            path: path_util::SafeRelativeUtf8UnixPathBuf::try_from(
                path.clone(),
            )
            .map_err(|e| invalid(&e.to_string()))?,
            hashes: result
                .snapshot
                .integrity
                .hashes
                .iter()
                .filter_map(|h| match h.algorithm {
                    HashAlgorithm::Sha1 => {
                        Some((PackFileHash::Sha1, h.value.clone()))
                    }
                    HashAlgorithm::Sha512 => {
                        Some((PackFileHash::Sha512, h.value.clone()))
                    }
                    _ => None,
                })
                .collect(),
            env: environment,
            downloads: urls.clone(),
            file_size: result
                .snapshot
                .integrity
                .size
                .unwrap()
                .try_into()
                .map_err(|_| invalid("file too large for legacy MR writer"))?,
        });
    }
    Ok(PackFormat {
        game: "minecraft".into(),
        format_version: 1,
        version_id: info.version,
        name: info.name,
        summary: (!info.summary.is_empty()).then_some(info.summary),
        files,
        dependencies,
    })
}

pub async fn execute(
    service: &Arc<ResourceService>,
    session: SessionId,
    revision: u32,
    choices: Vec<ExportEmbeddedFailureChoice>,
    info: ExportPackageInfo,
    output: PathBuf,
    cancel: &CancellationToken,
) -> ResourceResult<()> {
    let draft = service.export_draft(&session)?;
    let draft = draft.lock().await;
    validate_choices(&draft, revision, &choices)?;
    if info.name.trim().is_empty() || info.version.trim().is_empty() {
        return Err(invalid("package name and version are required"));
    }
    let root =
        crate::api::instance::get_full_path(&draft.scan.instance_id).await?;
    let resolved_output = if output.exists() {
        std::fs::canonicalize(&output).map_err(|e| invalid(&e.to_string()))?
    } else {
        let parent = output
            .parent()
            .ok_or_else(|| invalid("output has no parent directory"))?;
        std::fs::canonicalize(parent)
            .map_err(|e| invalid(&e.to_string()))?
            .join(
                output
                    .file_name()
                    .ok_or_else(|| invalid("output has no filename"))?,
            )
    };
    for path in &draft.selected {
        if std::fs::canonicalize(root.join(path))
            .is_ok_and(|path| path == resolved_output)
        {
            return Err(invalid(
                "the output cannot replace a selected source file",
            ));
        }
    }
    let state = crate::State::get().await?;
    let current =
        crate::state::get_instance(&draft.scan.instance_id, &state.pool)
            .await?
            .ok_or_else(|| invalid("instance no longer exists"))?;
    if current.applied_content_set.game_version
        != draft.scan.requirements.game_version
        || current.applied_content_set.loader_version
            != draft.scan.requirements.loader_version
        || (current.applied_content_set.loader
            != crate::state::ModLoader::Vanilla)
            .then(|| current.applied_content_set.loader.as_str().to_string())
            != draft.scan.requirements.loader
    {
        return Err(invalid(
            "game requirements changed after export analysis; reopen the export window",
        ));
    }
    for path in &draft.selected {
        let original = &draft.results[path].snapshot;
        let current = freeze(&root, original, cancel).await?.0;
        if current.integrity != original.integrity {
            return Err(invalid(
                "file changed after export analysis; refresh again",
            ));
        }
    }
    let pack = mr_pack(&draft, info, &choices)?;
    crate::api::instance::export_mrpack_prepared(
        &draft.scan.instance_id,
        output,
        draft.selected.clone(),
        pack,
    )
    .await?;
    Ok(())
}
