use super::{
    formats,
    plan::{invalid, snapshot},
    refresh::freeze,
};
use crate::resources::*;
use async_zip::{
    Compression, DeflateOption, ZipEntryBuilder, tokio::write::ZipFileWriter,
};
use futures::io::AsyncWriteExt;
use sha2::{Digest, Sha512};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

fn canceled(cancel: &CancellationToken) -> ResourceResult<()> {
    if cancel.is_cancelled() {
        Err(ResourceError::Canceled)
    } else {
        Ok(())
    }
}

pub fn temporary_path(
    plan: &ResourceExportPlan,
    output: &Path,
) -> ResourceResult<PathBuf> {
    let id = uuid::Uuid::parse_str(plan.id.as_str())
        .map_err(|e| invalid(e.to_string()))?;
    Ok(output
        .parent()
        .ok_or_else(|| invalid("output has no parent directory"))?
        .join(format!(".axolotl-export-{id}.tmp")))
}

pub async fn cleanup_partial(
    plan: &ResourceExportPlan,
    output: &Path,
) -> ResourceResult<()> {
    let temporary = temporary_path(plan, output)?;
    match tokio::fs::symlink_metadata(&temporary).await {
        Ok(meta) if meta.is_file() && !super::scan::linked(&meta) => {
            tokio::fs::remove_file(&temporary)
                .await
                .map_err(|e| invalid(e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(invalid("invalid temporary export file")),
        Err(e) => Err(invalid(e.to_string())),
    }
}

pub async fn validate_output(
    root: &Path,
    plan: &ResourceExportPlan,
    output: &Path,
) -> ResourceResult<PathBuf> {
    let parent = output
        .parent()
        .ok_or_else(|| invalid("output has no parent directory"))?;
    let parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|e| invalid(e.to_string()))?;
    let mut output = parent.join(
        output
            .file_name()
            .ok_or_else(|| invalid("output has no filename"))?,
    );
    if let Ok(meta) = tokio::fs::symlink_metadata(&output).await {
        if !meta.is_file() || super::scan::linked(&meta) {
            return Err(invalid("output must be a regular file"));
        }
        output = tokio::fs::canonicalize(&output)
            .await
            .map_err(|e| invalid(e.to_string()))?;
    }
    for action in &plan.files {
        if tokio::fs::canonicalize(root.join(&snapshot(action).relative_path))
            .await
            .is_ok_and(|source| source == output)
        {
            return Err(invalid(
                "the output cannot replace a selected source file",
            ));
        }
    }
    Ok(output)
}

pub async fn validate_files(
    root: &Path,
    plan: &ResourceExportPlan,
    cancel: &CancellationToken,
) -> ResourceResult<()> {
    let mut paths = HashSet::new();
    for action in &plan.files {
        let original = snapshot(action);
        if !paths.insert(&original.relative_path) {
            return Err(invalid("duplicate export path"));
        }
        if freeze(root, original, cancel).await?.0.integrity
            != original.integrity
        {
            return Err(invalid(format!(
                "{} changed after export analysis; refresh again",
                original.relative_path
            )));
        }
    }
    Ok(())
}

/// Writes only frozen actions, checks the bytes written, then atomically publishes.
/// The temporary file guard removes incomplete output on failure or cancellation.
pub async fn write(
    root: &Path,
    plan: &ResourceExportPlan,
    output: &Path,
    cancel: &CancellationToken,
    progress: &(impl Fn(u64, u64) + Send + Sync),
) -> ResourceResult<()> {
    write_checked(root, plan, output, cancel, progress, || async { Ok(()) })
        .await
}

pub async fn write_checked<F, Fut>(
    root: &Path,
    plan: &ResourceExportPlan,
    output: &Path,
    cancel: &CancellationToken,
    progress: &(impl Fn(u64, u64) + Send + Sync),
    validate: F,
) -> ResourceResult<()>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ResourceResult<()>>,
{
    validate().await?;
    if plan.package_info.name.trim().is_empty()
        || plan.package_info.version.trim().is_empty()
        || plan.requirements.game_version.is_empty()
    {
        return Err(invalid("invalid export package information"));
    }
    let (manifest_name, manifest) = formats::manifest(plan)?;
    let output = validate_output(root, plan, output).await?;
    validate_files(root, plan, cancel).await?;
    canceled(cancel)?;
    let temporary_file = temporary_path(plan, &output)?;
    if plan
        .files
        .iter()
        .any(|a| root.join(&snapshot(a).relative_path) == temporary_file)
    {
        return Err(invalid("temporary output is part of the selected files"));
    }
    let opened = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_file)
        .await
        .map_err(|e| invalid(e.to_string()))?;
    let temporary = tempfile::TempPath::try_from_path(temporary_file)
        .map_err(|e| invalid(e.to_string()))?;
    let mut file = opened;
    let mut zip = ZipFileWriter::with_tokio(&mut file);
    let total: u64 = plan
        .files
        .iter()
        .filter_map(|a| match a {
            ExportFileAction::Embed { snapshot } => snapshot.integrity.size,
            _ => None,
        })
        .fold(0u64, u64::saturating_add);
    let mut completed = 0;
    let mut buffer = vec![0; 256 * 1024];
    for action in &plan.files {
        canceled(cancel)?;
        let ExportFileAction::Embed { snapshot } = action else {
            continue;
        };
        let path = root.join(&snapshot.relative_path);
        let compression = match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "jar" | "zip" | "rar" | "7z" | "png" | "jpg" | "jpeg" | "ogg"
            | "mp4" | "gz" => Compression::Stored,
            _ => Compression::Deflate,
        };
        let builder = ZipEntryBuilder::new(
            format!("overrides/{}", snapshot.relative_path).into(),
            compression,
        )
        .deflate_option(DeflateOption::Other(1));
        let mut entry = zip
            .write_entry_stream(builder)
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let mut input = tokio::fs::File::open(&path)
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let mut sha512 = Sha512::new();
        let mut size = 0;
        loop {
            let read = tokio::select! { biased; _ = cancel.cancelled() => return Err(ResourceError::Canceled), r = input.read(&mut buffer) => r.map_err(|e| invalid(e.to_string()))? };
            if read == 0 {
                break;
            }
            entry
                .write_all(&buffer[..read])
                .await
                .map_err(|e| invalid(e.to_string()))?;
            sha512.update(&buffer[..read]);
            size += read as u64;
            completed += read as u64;
            progress(completed, total);
        }
        let hash = format!("{:x}", sha512.finalize());
        if snapshot.integrity.size != Some(size)
            || !snapshot.integrity.hashes.iter().any(|h| {
                h.algorithm == HashAlgorithm::Sha512
                    && h.value.eq_ignore_ascii_case(&hash)
            })
        {
            return Err(invalid(format!(
                "{} changed while being exported; refresh again",
                snapshot.relative_path
            )));
        }
        entry.close().await.map_err(|e| invalid(e.to_string()))?;
    }
    canceled(cancel)?;
    zip.write_entry_whole(
        ZipEntryBuilder::new(manifest_name.into(), Compression::Deflate),
        &serde_json::to_vec_pretty(&manifest)
            .map_err(|e| invalid(e.to_string()))?,
    )
    .await
    .map_err(|e| invalid(e.to_string()))?;
    zip.close().await.map_err(|e| invalid(e.to_string()))?;
    file.sync_all().await.map_err(|e| invalid(e.to_string()))?;
    drop(file);
    validate_files(root, plan, cancel).await?;
    validate_output(root, plan, &output).await?;
    validate().await?;
    canceled(cancel)?;
    temporary
        .persist(&output)
        .map_err(|e| invalid(e.to_string()))?;
    progress(total, total);
    Ok(())
}
