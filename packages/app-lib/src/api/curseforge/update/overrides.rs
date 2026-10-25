//! CurseForge modpack update overrides.

use super::*;
use crate::ErrorKind;
use crate::api::curseforge::dto::CurseForgeModpackManifest;
use crate::api::curseforge::modpack::CurseForgePackExpectedOverride;
use crate::util::download::constants::OVERRIDE_EXTRACTION_CONCURRENCY;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

pub(crate) struct ModpackOverrideTask {
    pub(crate) index: usize,
    pub(crate) target: PathBuf,
    pub(crate) size: u64,
}

pub(crate) fn collect_modpack_override_tasks<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &CurseForgeModpackManifest,
    instance_path: &Path,
    cancellation: Option<&CancellationToken>,
) -> crate::Result<Vec<ModpackOverrideTask>> {
    let prefix = format!("{}/", manifest.overrides.trim_matches('/'));
    let mut tasks_by_target = HashMap::<PathBuf, ModpackOverrideTask>::new();
    for index in 0..archive.len() {
        crate::api::pack::archive_util::check_cancellation(cancellation)?;
        let entry = archive.by_index(index).map_err(modpack_zip_error)?;
        let entry_name =
            crate::pack::detect::decode_zip_entry_name(entry.name_raw());
        if entry.is_dir() || !entry_name.starts_with(&prefix) {
            continue;
        }
        let relative = &entry_name[prefix.len()..];
        let safe_path = safe_archive_relative_path(relative)?;
        let target = instance_path.join(safe_path);
        // Preserve archive order semantics for duplicate targets: the last
        // entry wins, while unique targets can be extracted independently.
        tasks_by_target.insert(
            target.clone(),
            ModpackOverrideTask {
                index,
                target,
                size: entry.size(),
            },
        );
    }
    let total_size = tasks_by_target
        .values()
        .fold(0_u64, |total, task| total.saturating_add(task.size));
    let limit = crate::api::pack::archive_util::EXTRACTION_SIZE_LIMIT;
    if total_size > limit {
        return Err(ErrorKind::InputError(format!(
            "CurseForge modpack overrides exceed the extraction limit: {total_size} unpacked bytes required, limit is {limit} bytes"
        ))
        .into());
    }
    Ok(tasks_by_target.into_values().collect())
}

pub(crate) fn materialize_modpack_overrides(
    archive_path: &Path,
    instance_path: &Path,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> crate::Result<(
    u32,
    crate::api::pack::archive_util::StagedArchiveReplacements,
)> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(modpack_zip_error)?;
    let manifest = read_modpack_manifest(&mut archive)?;
    let tasks = collect_modpack_override_tasks(
        &mut archive,
        &manifest,
        instance_path,
        cancellation,
    )?;
    drop(archive);
    if tasks.is_empty() {
        return Ok((
            0,
            crate::api::pack::archive_util::materialize_staged_archive_entries(
                &[],
                cancellation,
            )?,
        ));
    }
    let targets = tasks
        .iter()
        .map(|task| task.target.clone())
        .collect::<Vec<_>>();
    let worker_count = OVERRIDE_EXTRACTION_CONCURRENCY.min(tasks.len());
    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let extraction_result = std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            workers.push(scope.spawn(move || -> crate::Result<u32> {
                let file = std::fs::File::open(archive_path).map_err(|error| {
                    crate::util::io::IOError::with_path(error, archive_path)
                })?;
                let mut archive =
                    zip::ZipArchive::new(file).map_err(modpack_zip_error)?;
                let mut files_written = 0_u32;
                loop {
                    crate::api::pack::archive_util::check_cancellation(
                        cancellation,
                    )?;
                    let Some(task) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let mut entry = archive
                        .by_index(task.index)
                        .map_err(modpack_zip_error)?;
                    let written = crate::api::pack::archive_util::write_archive_entry_to_staging(
                        &mut entry,
                        &task.target,
                        cancellation,
                    )?;
                    if written != entry.size() {
                        return Err(ErrorKind::InputError(
                            "CurseForge modpack override was truncated during extraction"
                                .to_string(),
                        )
                        .into());
                    }
                    files_written = files_written.checked_add(1).ok_or_else(|| {
                        ErrorKind::InputError(
                            "CurseForge modpack contains too many override files"
                                .to_string(),
                        )
                    })?;
                }
                Ok(files_written)
            }));
        }
        let mut files_written = 0_u32;
        for worker in workers {
            let count = worker.join().map_err(|_| {
                ErrorKind::OtherError(
                    "CurseForge override extraction worker panicked"
                        .to_string(),
                )
            })??;
            files_written =
                files_written.checked_add(count).ok_or_else(|| {
                    ErrorKind::InputError(
                        "CurseForge modpack contains too many override files"
                            .to_string(),
                    )
                })?;
        }
        Ok(files_written)
    });
    match extraction_result {
        Ok(files_written) => {
            let replacements = crate::api::pack::archive_util::materialize_staged_archive_entries(
                &targets,
                cancellation,
            )?;
            Ok((files_written, replacements))
        }
        Err(error) => {
            if let Err(cleanup_error) =
                crate::api::pack::archive_util::discard_staged_archive_entries(
                    &targets,
                )
            {
                return Err(ErrorKind::OtherError(format!(
                    "{error}; failed to clean staged CurseForge overrides: {cleanup_error}"
                ))
                .into());
            }
            Err(error)
        }
    }
}

#[cfg(test)]
pub(crate) fn extract_modpack_overrides(
    archive_path: &Path,
    instance_path: &Path,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> crate::Result<u32> {
    let (files_written, replacements) = materialize_modpack_overrides(
        archive_path,
        instance_path,
        cancellation,
    )?;
    replacements.finalize()?;
    Ok(files_written)
}

pub(crate) async fn settle_materialized_modpack_overrides(
    instance_id: String,
    cancellation: CancellationToken,
    replacements: crate::api::pack::archive_util::StagedArchiveReplacements,
    commit: bool,
) -> crate::Result<()> {
    crate::api::pack::archive_util::run_blocking_instance_write(
        instance_id,
        cancellation,
        move |_| {
            if commit {
                replacements.finalize()
            } else {
                replacements.rollback()
            }
        },
    )
    .await
}

pub(crate) fn read_modpack_override_content<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &CurseForgeModpackManifest,
) -> crate::Result<Vec<CurseForgePackExpectedOverride>> {
    let prefix = format!("{}/", manifest.overrides.trim_matches('/'));
    let mut overrides = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(modpack_zip_error)?;
        let entry_name =
            crate::pack::detect::decode_zip_entry_name(entry.name_raw());
        if entry.is_dir() || !entry_name.starts_with(&prefix) {
            continue;
        }
        let Some(override_content) =
            curseforge_override_content(&entry_name[prefix.len()..])?
        else {
            continue;
        };
        overrides.push(override_content);
    }
    overrides.sort_by(|left, right| {
        left.expected_relative_path
            .cmp(&right.expected_relative_path)
    });
    overrides.dedup_by(|left, right| {
        left.expected_relative_path == right.expected_relative_path
    });
    Ok(overrides)
}

pub(crate) fn curseforge_override_content(
    relative_path: &str,
) -> crate::Result<Option<CurseForgePackExpectedOverride>> {
    let relative_path = safe_archive_relative_path(relative_path)?;
    let Some(project_type) = crate::state::instances::adapters::filesystem::project_type_from_relative_path(
        &relative_path,
    ) else {
        return Ok(None);
    };
    if !crate::state::instances::adapters::filesystem::is_scannable_project_path(
        project_type,
        &relative_path,
    ) {
        return Ok(None);
    }
    Ok(Some(CurseForgePackExpectedOverride {
        project_type,
        expected_relative_path: relative_path,
    }))
}

pub(crate) fn safe_archive_relative_path(value: &str) -> crate::Result<String> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ErrorKind::InputError(
            "CurseForge modpack contains an invalid override path".to_string(),
        )
        .into());
    }
    Ok(normalized)
}
