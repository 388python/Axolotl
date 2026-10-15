//! Metadata reachability uses only local files and read-only channel databases.

use super::*;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde_json::Value;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tokio::sync::Mutex;

static PLANS: LazyLock<Mutex<HashMap<String, CleanupPlan>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const PLAN_AGE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Serialize)]
pub struct MetaGroup {
    pub id: String,
    pub category: String,
    pub name: String,
    pub used: u64,
    pub unused: u64,
    pub unknown: u64,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetaAnalysis {
    pub id: String,
    pub scanned_at: DateTime<Utc>,
    pub groups: Vec<MetaGroup>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MetaCleanupResult {
    pub removed_files: u64,
    pub removed_bytes: u64,
    pub skipped_files: u64,
    pub failures: Vec<String>,
}

struct CleanupPlan {
    created: std::time::Instant,
    root: PathBuf,
    files: Vec<Candidate>,
}

struct Candidate {
    relative: String,
    group: String,
    size: u64,
    modified: Option<cap_std::time::SystemTime>,
    created: Option<cap_std::time::SystemTime>,
}

#[derive(Default)]
struct References {
    paths: HashSet<String>,
    versions: HashSet<String>,
    manifests: HashSet<String>,
    natives: HashSet<String>,
    blocked: bool,
    busy: bool,
    warnings: Vec<String>,
}

fn key(value: &str) -> String {
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.contains(':')
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn component(value: &str) -> bool {
    safe_relative(value) && !value.contains('/')
}

impl References {
    fn keep(&mut self, path: String) -> crate::Result<()> {
        if !safe_relative(&path) {
            return Err(invalid("Invalid path in local metadata"));
        }
        self.paths.insert(key(&path));
        Ok(())
    }

    fn used(&self, relative: &str) -> Option<bool> {
        if self.blocked || self.busy {
            return None;
        }
        if relative.starts_with("libraries/")
            && self.paths.contains("libraries/*")
        {
            return None;
        }
        let parts: Vec<_> = relative.split('/').collect();
        match parts.as_slice() {
            ["versions", version, name] => {
                if self.versions.contains(&key(version)) {
                    return Some(true);
                }
                (name == &format!("{version}.jar")
                    || name == &format!("{version}.json"))
                    .then_some(false)
            }
            ["natives", version, ..] => {
                Some(self.natives.contains(&key(version)))
            }
            ["libraries" | "assets" | "log_configs" | "resources", ..]
                if self.paths.contains(&key(relative)) =>
            {
                Some(true)
            }
            ["libraries", ..]
                if parts.len() >= 5 && relative.ends_with(".jar") =>
            {
                Some(false)
            }
            ["assets", "objects", prefix, hash]
                if hash.len() == 40
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && *prefix == &hash[..2] =>
            {
                Some(false)
            }
            ["assets", "indexes", name] if name.ends_with(".json") => {
                Some(false)
            }
            ["log_configs", name] if name.ends_with(".xml") => Some(false),
            _ => None,
        }
    }
}

fn invalid(message: impl Into<String>) -> crate::Error {
    crate::ErrorKind::InputError(message.into()).into()
}

async fn read_json(path: &Path) -> crate::Result<Value> {
    let metadata = fs::symlink_metadata(path).await?;
    if crate::util::io::is_symlink_or_reparse(&metadata)
        || !metadata.is_file()
        || metadata.len() > 64 * 1024 * 1024
    {
        return Err(invalid(format!(
            "Cannot read local metadata: {}",
            path.display()
        )));
    }
    Ok(serde_json::from_slice(&fs::read(path).await?)?)
}

async fn channel_references(
    pool: &SqlitePool,
    refs: &mut References,
    directories: &DirectoryInfo,
) -> crate::Result<()> {
    let busy: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM install_jobs WHERE status IN ('queued', 'running', 'canceling', 'waiting_for_user') OR json_extract(state, '$.rollback') IS NOT NULL").fetch_one(pool).await?;
    refs.busy |= busy > 0;
    let overrides: Vec<String> = sqlx::query_scalar(
        "SELECT json(overrides) FROM instance_launch_overrides",
    )
    .fetch_all(pool)
    .await?;
    let settings: Vec<String> =
        sqlx::query_scalar("SELECT json(extra_launch_args) FROM settings")
            .fetch_all(pool)
            .await?;
    if overrides.iter().chain(settings.iter()).any(|value| {
        let value = value.to_lowercase();
        value.contains(".jar")
            || value.contains(".zip")
            || value.contains("meta/")
            || value.contains("meta\\")
    }) {
        refs.blocked = true;
        refs.warnings.push("Custom launch arguments may reference metadata files; their usage cannot be established automatically.".into());
    }
    let instances = crate::state::instances::adapters::sqlite::instance_rows::list_instances(pool).await?;
    for instance in instances {
        let runtime = crate::launcher::instance_runtime::InstanceRuntimeAdapter::for_instance(&instance, directories)?;
        if let Some(direct) = runtime.direct_link() {
            let meta_root = identity(&directories.metadata_dir());
            if [
                direct.libraries_dir(),
                direct.assets_dir(),
                direct.version_dir(),
            ]
            .iter()
            .any(|path| identity(path).starts_with(&meta_root))
            {
                refs.blocked = true;
                refs.warnings.push("An external instance uses the launcher's metadata directory; shared usage cannot be fully confirmed.".into());
            }
            refs.natives.insert(key(&direct.natives_cache_key()));
            continue;
        }
        let rows = sqlx::query("SELECT game_version, loader, loader_version FROM instance_content_sets WHERE instance_id = ?").bind(&instance.id).fetch_all(pool).await?;
        if rows.is_empty() {
            return Err(invalid("An instance has no local content version"));
        }
        for row in rows {
            let game: String = row.try_get("game_version")?;
            let loader: String = row.try_get("loader")?;
            let version: Option<String> = row.try_get("loader_version")?;
            if !component(&game) {
                return Err(invalid("Unresolved game version"));
            }
            refs.versions.insert(key(&game));
            if loader != "vanilla" {
                let version = version
                    .filter(|version| {
                        component(version)
                            && version != "latest"
                            && version != "stable"
                    })
                    .ok_or_else(|| {
                        invalid("An instance has no exact loader version")
                    })?;
                let combined = format!("{game}-{version}");
                refs.versions.insert(key(&combined));
                refs.manifests.insert(key(&combined));
                refs.natives.insert(key(&combined));
            } else {
                refs.natives.insert(key(&game));
                refs.manifests.insert(key(&game));
            }
        }
    }
    Ok(())
}

async fn references(directories: &DirectoryInfo) -> References {
    let mut refs = References::default();
    let mut databases = 0;
    for path in [
        directories.settings_dir.join("release/app.db"),
        directories.settings_dir.join("beta/app.db"),
        directories.settings_dir.join("app.db"),
    ] {
        match fs::try_exists(&path).await {
            Ok(false) => continue,
            Err(error) => {
                refs.blocked = true;
                refs.warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
            Ok(true) => {}
        }
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .create_if_missing(false);
        match SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
        {
            Ok(pool) => {
                databases += 1;
                if let Err(error) =
                    channel_references(&pool, &mut refs, directories).await
                {
                    refs.blocked = true;
                    refs.warnings.push(format!("{}: {error}", path.display()));
                }
                pool.close().await;
            }
            Err(error) => {
                refs.blocked = true;
                refs.warnings.push(format!("{}: {error}", path.display()));
            }
        }
    }
    // A missing active database must never turn all metadata into orphans.
    if databases == 0 || crate::State::get().await.is_err() {
        refs.blocked = true;
    }
    let versions: Vec<_> = refs.manifests.iter().cloned().collect();
    for version in versions {
        if let Err(error) =
            version_references(&directories.metadata_dir(), &version, &mut refs)
                .await
        {
            refs.blocked = true;
            refs.warnings.push(format!("{version}: {error}"));
        }
    }
    if refs.busy {
        refs.warnings.push("An installation or rollback is pending. Finish it before cleaning metadata.".into());
    }
    refs
}

async fn version_references(
    root: &Path,
    version: &str,
    refs: &mut References,
) -> crate::Result<()> {
    let path = root
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    let value = read_json(&path).await?;
    if value
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(|id| key(id) != key(version))
    {
        return Err(invalid("Local version identity mismatch"));
    }
    if value.get("inheritsFrom").is_some() {
        return Err(invalid("Unresolved local version inheritance"));
    }
    let info: daedalus::minecraft::VersionInfo =
        serde_json::from_value(value.clone())?;
    for library in &info.libraries {
        refs.keep(format!(
            "libraries/{}",
            daedalus::get_path_from_artifact(&library.name)?
        ))?;
        if let Some(natives) = &library.natives {
            for classifier in natives.values() {
                for width in ["32", "64"] {
                    let classifier = classifier.replace("${arch}", width);
                    refs.keep(format!("libraries/{}", crate::launcher::download::native_library_artifact_path(library, &classifier)?))?;
                }
            }
        }
        if let Some(downloads) = &library.downloads {
            if let Some(classifiers) = &downloads.classifiers {
                for classifier in classifiers.keys() {
                    refs.keep(format!("libraries/{}", crate::launcher::download::native_library_artifact_path(library, classifier)?))?;
                }
            }
            for artifact in downloads.artifact.iter().chain(
                downloads
                    .classifiers
                    .iter()
                    .flat_map(|classifiers| classifiers.values()),
            ) {
                if let Some(path) = &artifact.path {
                    refs.keep(format!("libraries/{path}"))?;
                }
            }
        }
    }
    // Processor inputs, outputs and data templates can reference generated JARs.
    // Until all templates can be resolved locally, retain the shared library tree.
    if value
        .get("processors")
        .and_then(Value::as_array)
        .is_some_and(|processors| !processors.is_empty())
    {
        refs.paths.insert("libraries/*".into());
        refs.warnings.push(format!("{version}: processor-generated library usage cannot be fully confirmed; shared libraries are kept."));
    }
    let index_id = &info.asset_index.id;
    if !component(index_id) {
        return Err(invalid("Invalid asset index ID"));
    }
    refs.keep(format!("assets/indexes/{index_id}.json"))?;
    let index = read_json(
        &root.join("assets/indexes").join(format!("{index_id}.json")),
    )
    .await?;
    let objects = index
        .get("objects")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("Invalid asset index"))?;
    for (name, asset) in objects {
        let hash = asset
            .get("hash")
            .and_then(Value::as_str)
            .filter(|hash| {
                hash.len() == 40
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| invalid("Invalid asset hash"))?;
        refs.keep(format!("assets/objects/{}/{hash}", &hash[..2]))?;
        if info.assets == "legacy" {
            refs.keep(format!("resources/{name}"))?;
        }
        if index.get("virtual").and_then(Value::as_bool) == Some(true) {
            refs.keep(format!("assets/virtual/{index_id}/{name}"))?;
        }
        if index.get("map_to_resources").and_then(Value::as_bool) == Some(true)
        {
            refs.keep(format!("resources/{name}"))?;
        }
    }
    if let Some(logging) = value.get("logging").and_then(Value::as_object) {
        for config in logging.values() {
            if let Some(id) = config.pointer("/file/id").and_then(Value::as_str)
            {
                if !component(id) {
                    return Err(invalid("Invalid logging filename"));
                }
                refs.keep(format!("log_configs/{id}"))?;
            }
        }
    }
    Ok(())
}

fn group_for(relative: &str) -> (String, String, String) {
    let mut parts = relative.split('/');
    let category = parts.next().unwrap_or("other").to_owned();
    let name = match category.as_str() {
        "versions" | "natives" | "java_versions" => {
            parts.next().unwrap_or_default().to_owned()
        }
        _ => String::new(),
    };
    let id = if name.is_empty() {
        category.clone()
    } else {
        format!("{category}/{name}")
    };
    (id, category, name)
}

fn directory(root: &Dir, relative: &str) -> std::io::Result<Dir> {
    let mut dir = root.try_clone()?;
    if relative.is_empty() {
        return Ok(dir);
    }
    if !safe_relative(relative) {
        return Err(std::io::Error::other("Invalid metadata path"));
    }
    for part in relative.split('/') {
        dir = dir.open_dir_nofollow(part)?;
    }
    Ok(dir)
}

fn opened_root(directories: &DirectoryInfo) -> std::io::Result<Dir> {
    Dir::open_ambient_dir(
        &directories.config_dir,
        cap_std::ambient_authority(),
    )?
    .open_dir_nofollow("meta")
}

fn collect(
    root: &Dir,
    relative: &str,
    refs: &References,
    groups: &mut BTreeMap<String, MetaGroup>,
    files: &mut Vec<Candidate>,
    warnings: &mut Vec<String>,
) -> std::io::Result<()> {
    let dir = directory(root, relative)?;
    for entry in dir.entries()? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if !safe_relative(&path) {
            warnings.push(path);
            continue;
        }
        let metadata = dir.symlink_metadata(entry.file_name())?;
        if metadata.is_symlink() {
            warnings.push(path);
            continue;
        }
        if metadata.is_dir() {
            if let Err(error) =
                collect(root, &path, refs, groups, files, warnings)
            {
                warnings.push(format!("{path}: {error}"));
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let (id, category, name) = group_for(&path);
        let group = groups.entry(id.clone()).or_insert_with(|| MetaGroup {
            id: id.clone(),
            category,
            name,
            used: 0,
            unused: 0,
            unknown: 0,
            files: 0,
        });
        group.files += 1;
        let size = metadata.len();
        let used = refs.used(&path);
        match used {
            Some(true) => group.used += size,
            Some(false) => {
                group.unused += size;
                files.push(Candidate {
                    relative: path,
                    group: id,
                    size,
                    modified: metadata.modified().ok(),
                    created: metadata.created().ok(),
                });
            }
            None => group.unknown += size,
        }
    }
    Ok(())
}

pub async fn analyze_meta() -> crate::Result<MetaAnalysis> {
    let _usage = usage::lease().await?;
    let directories = dirs()?;
    let refs = references(&directories).await;
    let root_path = identity(&directories.metadata_dir());
    let mut warnings = refs.warnings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut groups = BTreeMap::new();
        let mut files = Vec::new();
        match opened_root(&directories) {
            Ok(root) => collect(
                &root,
                "",
                &refs,
                &mut groups,
                &mut files,
                &mut warnings,
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok::<_, std::io::Error>((
            groups.into_values().collect::<Vec<_>>(),
            files,
            warnings,
        ))
    })
    .await
    .map_err(|error| invalid(error.to_string()))??;
    let (groups, files, mut warnings) = result;
    warnings.truncate(100);
    let id = uuid::Uuid::new_v4().to_string();
    let mut plans = PLANS.lock().await;
    plans.retain(|_, plan| plan.created.elapsed() < PLAN_AGE);
    if plans.len() >= 4 {
        plans.clear();
    }
    plans.insert(
        id.clone(),
        CleanupPlan {
            created: std::time::Instant::now(),
            root: root_path,
            files,
        },
    );
    Ok(MetaAnalysis {
        id,
        scanned_at: Utc::now(),
        groups,
        warnings,
    })
}

pub async fn clean_meta(
    id: String,
    groups: Vec<String>,
) -> crate::Result<MetaCleanupResult> {
    let _scan = SCAN_LOCK.lock().await;
    let usage = usage::exclusive().await?;
    let state = crate::State::get().await?;
    if !state.process_manager.get_all().is_empty() {
        return Err(invalid("Stop running games before cleaning metadata"));
    }
    let directories = dirs()?;
    let refs = references(&directories).await;
    if refs.blocked || refs.busy {
        return Err(invalid(
            "Metadata references could not be verified or installation recovery is pending. Refresh the analysis after resolving it.",
        ));
    }
    let mut plans = PLANS.lock().await;
    let plan = plans
        .get(&id)
        .ok_or_else(|| invalid("Metadata analysis expired. Analyze again."))?;
    if plan.created.elapsed() >= PLAN_AGE
        || plan.root != identity(&directories.metadata_dir())
    {
        return Err(invalid("Metadata analysis expired. Analyze again."));
    }
    let selection: HashSet<_> = groups.into_iter().collect();
    if selection.is_empty()
        || selection
            .iter()
            .any(|group| !plan.files.iter().any(|file| &file.group == group))
    {
        return Err(invalid(
            "Select unused metadata from the current analysis",
        ));
    }
    let plan = plans.remove(&id).unwrap();
    drop(plans);
    let result = tokio::task::spawn_blocking(move || {
        let _usage = usage;
        remove_candidates(&directories, plan.files, &selection, &refs)
    })
    .await
    .map_err(|error| invalid(error.to_string()))??;
    let _ =
        fs::remove_file(dirs()?.caches_dir().join(STORAGE_CACHE_FILE)).await;
    Ok(result)
}

fn remove_candidates(
    directories: &DirectoryInfo,
    files: Vec<Candidate>,
    selection: &HashSet<String>,
    refs: &References,
) -> std::io::Result<MetaCleanupResult> {
    let root = opened_root(&directories)?;
    let mut result = MetaCleanupResult::default();
    for file in files
        .into_iter()
        .filter(|file| selection.contains(&file.group))
    {
        if refs.used(&file.relative) != Some(false)
            || (file.relative.starts_with("libraries/")
                && refs.paths.contains("libraries/*"))
        {
            result.skipped_files += 1;
            continue;
        }
        let outcome = (|| {
            let (parent, name) = file
                .relative
                .rsplit_once('/')
                .ok_or_else(|| std::io::Error::other("Invalid candidate"))?;
            let dir = directory(&root, parent)?;
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let handle = dir.open_with(name, &options)?;
            let metadata = handle.metadata()?;
            if !metadata.is_file()
                || metadata.len() != file.size
                || metadata.modified().ok() != file.modified
                || metadata.created().ok() != file.created
            {
                return Ok(false);
            }
            drop(handle);
            dir.remove_file(name)?;
            Ok::<_, std::io::Error>(true)
        })();
        match outcome {
            Ok(true) => {
                result.removed_files += 1;
                result.removed_bytes += file.size;
            }
            Ok(false) => result.skipped_files += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                result.skipped_files += 1
            }
            Err(error) => {
                if result.failures.len() < 100 {
                    result.failures.push(format!("{}: {error}", file.relative));
                }
            }
        }
    }
    Ok::<_, std::io::Error>(result)
}

#[cfg(test)]
mod tests;
