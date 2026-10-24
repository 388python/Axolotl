//! CurseForge integration: submodule wiring and the crate-internal re-exports.

mod auth;
mod catalog;
mod constants;
mod content_change;
mod download;
mod dto;
mod install;
mod manual;
mod mapping;
mod metrics;
mod modpack;
mod plans;
mod preview;
mod selection;
mod transport;
mod update;
mod validation;
mod verify;

#[cfg(test)]
mod tests;

pub use self::auth::*;
pub use self::catalog::*;
pub use self::dto::*;
pub use self::install::*;
pub use self::manual::*;
pub use self::modpack::*;
pub use self::preview::*;
pub use self::selection::*;
pub use self::update::*;

pub(crate) use self::catalog::{
    derived_curseforge_download_url, resource_request,
};
pub(crate) use self::constants::{
    DEPENDENCY_RELATION_INCLUDE, DEPENDENCY_RELATION_REQUIRED,
    FABRIC_API_CURSEFORGE_PROJECT_ID, QUILTED_FABRIC_API_CURSEFORGE_PROJECT_ID,
};
pub(crate) use self::content_change::prepare_curseforge_content_change_action;
pub(crate) use self::dto::StagedCurseForgeUpgrade;
pub(crate) use self::dto::{
    CurseForgeManifestFile, CurseForgePackExpectedContent,
    CurseForgePackExpectedMember, CurseForgePackExpectedOverride,
    get_modpack_expected_members,
};
pub(crate) use self::dto::{
    apply_staged_curseforge_upgrade_file,
    apply_staged_curseforge_upgrade_file_with_state,
    stage_curseforge_upgrade_file,
};
pub(crate) use self::manual::{
    import_pending_manual_download_from_path,
    reconcile_persisted_curseforge_waiting_jobs,
    scan_pending_manual_downloads_in,
};
pub(crate) use self::modpack::install_local_manifest_files;
pub(crate) use self::transport::api_key;
pub(crate) use self::update::loader_family;
pub(crate) use self::update::update_managed_modpack_with_reporter;
pub(crate) use self::verify::{
    CurseForgeFingerprintHasher, compute_fingerprint,
};
