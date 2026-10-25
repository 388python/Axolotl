//! CurseForge modpack update target.

use crate::ErrorKind;
use crate::api::curseforge::dto::{
    CurseForgeInstallResult, CurseForgeModpackManifest, CurseForgeModpackTarget,
};
use crate::state::ModLoader;
use std::io::{Read, Seek};
use std::path::Path;

pub(crate) fn read_modpack_manifest<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> crate::Result<CurseForgeModpackManifest> {
    let mut entry = archive.by_name("manifest.json").map_err(|_| {
        ErrorKind::InputError(
            "CurseForge modpack is missing manifest.json".to_string(),
        )
    })?;
    let mut json = String::new();
    entry.read_to_string(&mut json)?;
    Ok(serde_json::from_str::<CurseForgeModpackManifest>(&json)?)
}

pub(crate) fn modpack_target(
    manifest: &CurseForgeModpackManifest,
) -> crate::Result<CurseForgeModpackTarget> {
    let Some(manifest_loader) = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
    else {
        return Ok(CurseForgeModpackTarget {
            game_version: manifest.minecraft.version.clone(),
            loader: ModLoader::Vanilla,
            loader_version: None,
        });
    };

    let family = loader_family(&manifest_loader.id);
    let manifest_loader_version = manifest_loader
        .id
        .strip_prefix(family)
        .and_then(|version| version.strip_prefix('-'))
        .filter(|version| !version.is_empty());
    let disguised_cleanroom = family == "forge"
        && manifest.minecraft.version == "1.12.2"
        && manifest_loader_version
            .is_some_and(|version| version.starts_with("0."));
    let loader = match family {
        "cleanroom" => ModLoader::Cleanroom,
        "forge" if disguised_cleanroom => ModLoader::Cleanroom,
        "forge" => ModLoader::Forge,
        "fabric" => ModLoader::Fabric,
        "quilt" => ModLoader::Quilt,
        "neo" | "neoforge" => ModLoader::NeoForge,
        _ => {
            return Err(ErrorKind::InputError(format!(
                "CurseForge modpack uses unsupported loader {}",
                manifest_loader.id
            ))
            .into());
        }
    };
    let loader_version = manifest_loader_version
        .filter(|version| !disguised_cleanroom || version.starts_with("0."))
        .map(str::to_string);

    Ok(CurseForgeModpackTarget {
        game_version: manifest.minecraft.version.clone(),
        loader,
        loader_version,
    })
}

/// Reads the identity fields from a local CurseForge archive before an
/// instance is created, avoiding the temporary default Vanilla metadata.
pub async fn get_local_modpack_target(
    archive_path: &Path,
) -> crate::Result<(String, CurseForgeModpackTarget)> {
    let path = archive_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(path)?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(modpack_zip_error)?;
        let manifest = read_modpack_manifest(&mut archive)?;
        let name = manifest
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "CurseForge Modpack".to_string());
        Ok((name, modpack_target(&manifest)?))
    })
    .await?
}

pub(crate) fn modpack_zip_error(error: zip::result::ZipError) -> crate::Error {
    ErrorKind::InputError(format!(
        "CurseForge modpack archive is invalid: {error}"
    ))
    .into()
}

pub(crate) fn loader_family(loader_id: &str) -> &str {
    loader_id.split('-').next().unwrap_or(loader_id)
}

pub(crate) fn loader_type(loader: &str) -> Option<u32> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "forge" | "cleanroom" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

pub(crate) fn looks_like_game_version(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && loader_type(value).is_none()
}

pub(crate) fn merge_install_result(
    target: &mut CurseForgeInstallResult,
    mut source: CurseForgeInstallResult,
) {
    target.installed.append(&mut source.installed);
    target.manual_downloads.append(&mut source.manual_downloads);
    target.failed_downloads.append(&mut source.failed_downloads);
    target
        .optional_dependencies
        .append(&mut source.optional_dependencies);
    target
        .incompatible_dependencies
        .append(&mut source.incompatible_dependencies);
}
