use crate::api::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tauri::http::HeaderValue;
use tauri::http::header::ACCEPT;
use tauri::{Manager, ResourceId, Runtime, Webview};
use tauri_plugin_http::reqwest;
use tauri_plugin_http::reqwest::ClientBuilder;
use tauri_plugin_updater::{Error, Update, UpdaterExt};
use theseus::{
    LoadingBarType, emit_loading, init_loading, launcher_user_agent,
};
use tokio::time::Instant;
use url::Url;

const UPDATE_SERVER_LATEST_URL: &str = "https://update.axlmc.org/latest";
const UPDATE_SERVER_API: &str = "https://update.axlmc.org/api/versions";
const UPDATE_SERVER_BASE: &str = "https://update.axlmc.org/";

// The updater plugin builds `Update` with no request timeout, so a stalled
// connection would hang the download forever. Bound the whole download.
const UPDATE_DOWNLOAD_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(15 * 60);

// 降级源的 release 列表只是决定「去哪个 tag 取清单」，卡住它没有意义。
// 这个上限只约束 GitLink 列表请求本身，不参与是否降级的判断。
const UPDATE_CHECK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(15);

// 主源「慢」的判定阈值：成功但耗时超过它就只在下载前改选 GitLink 镜像主机。
// 它不参与签名校验（清单里的 signature 原值照旧由插件在下载完成后校验），
// 也不改变降级源的判定（是否降级仍然只看主源的错误 / None）。
// 3 秒是经验值，不读环境变量：没有实测依据的机器不该走另一条下载主机。
const UPDATE_MIRROR_LATENCY_THRESHOLD: std::time::Duration =
    std::time::Duration::from_secs(3);

// ── Shared types ─────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    force_update: bool,
    raw_json: serde_json::Value,
}

#[derive(Default)]
pub struct PendingUpdateData(pub Mutex<Option<(Arc<Update>, Vec<u8>)>>);

// ── Update Server API types ─────────────────────────────────────

#[derive(Deserialize)]
struct VersionsResponse {
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize)]
struct VersionEntry {
    version: String,
    artifacts: Vec<ArtifactEntry>,
}

#[derive(Deserialize)]
struct ArtifactEntry {
    kind: String,
    #[serde(default)]
    variant: Option<String>,
    platform: String,
    architecture: String,
    relative_path: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: u64,
}

/// The .deb asset for an apt-managed Linux update, from the Update Server
/// catalog (`/api/versions`). The deb has no minisign signature, so its
/// integrity is verified with the catalog's sha256 and size instead.
struct AptDebAsset {
    url: Url,
    sha256: String,
    size: u64,
}

fn apt_deb_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        arch => Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            format!("Unsupported architecture for apt updates: {arch}"),
        ))
        .into()),
    }
}

async fn fetch_apt_deb_asset(version: &str) -> Result<AptDebAsset> {
    let response = ClientBuilder::new()
        .user_agent(launcher_user_agent())
        .timeout(UPDATE_DOWNLOAD_TIMEOUT)
        .build()?
        .get(UPDATE_SERVER_API)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Failed to fetch update catalog: {}",
            response.status()
        ))
        .into());
    }

    let catalog: VersionsResponse = response.json().await?;
    let release = catalog
        .versions
        .iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Update catalog has no entry for version {version}"
            )))
        })?;

    let arch = std::env::consts::ARCH;
    let artifact = release
        .artifacts
        .iter()
        .find(|entry| {
            entry.kind == "installer"
                && entry.variant.as_deref() == Some("deb")
                && entry.platform == "linux"
                && entry.architecture == arch
        })
        .ok_or_else(|| {
            theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Update catalog has no deb artifact for {version} on {arch}"
            )))
        })?;

    let sha256 = artifact.sha256.clone().ok_or_else(|| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Update catalog has no sha256 for the deb artifact of {version}"
        )))
    })?;

    let url =
        Url::parse(&format!("{UPDATE_SERVER_BASE}{}", artifact.relative_path))
            .map_err(|error| {
                theseus::Error::from(theseus::ErrorKind::OtherError(
                    error.to_string(),
                ))
            })?;

    Ok(AptDebAsset {
        url,
        sha256,
        size: artifact.size,
    })
}

// ── Updater plugin helpers ───────────────────────────────────────

fn update_channel(channel: &str) -> Result<&str> {
    match channel {
        "release" | "beta" => Ok(channel),
        _ => Err(update_channel_error(channel)),
    }
}

fn update_channel_error(channel: &str) -> crate::api::TheseusSerializableError {
    let error = theseus::Error::from(theseus::ErrorKind::OtherError(format!(
        "Unknown update channel: {channel}"
    )));
    error.into()
}

fn update_platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-aarch64"),
        ("macos", "x86_64") => Ok("darwin-x86_64"),
        ("macos", "aarch64") => Ok("darwin-aarch64"),
        (os, arch) => {
            Err(theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Unsupported updater platform: {os}-{arch}"
            )))
            .into())
        }
    }
}

fn update_endpoint() -> Result<Url> {
    Url::parse(UPDATE_SERVER_LATEST_URL).map_err(|error| {
        theseus::Error::from(theseus::ErrorKind::OtherError(error.to_string()))
            .into()
    })
}

/// 记录一次更新源检查的结果。
///
/// 是否降级只看结果（错误 / `None`），不看耗时，因此这里不设任何超时阈值。
/// 耗时只决定主源成功时「下载前选哪个主机」（见
/// `UPDATE_MIRROR_LATENCY_THRESHOLD`），与本函数的判定无关。
fn log_check_outcome(
    source: &str,
    tag: Option<&str>,
    elapsed: std::time::Duration,
    failure: Option<&str>,
) {
    match failure {
        Some(failure) => tracing::warn!(
            source,
            tag = tag.unwrap_or_default(),
            elapsed_ms = elapsed.as_millis(),
            failure,
            "Update source check failed"
        ),
        None => tracing::info!(
            source,
            tag = tag.unwrap_or_default(),
            elapsed_ms = elapsed.as_millis(),
            "Update source check finished"
        ),
    }
}

/// Build the platform-updater with the given endpoints and run a check.
async fn check_with_endpoints<R: Runtime>(
    webview: &Webview<R>,
    channel: &str,
    endpoints: Vec<Url>,
) -> Result<Option<Update>> {
    let channel = update_channel(channel)?;
    let platform = update_platform()?;
    let current_version =
        webview.app_handle().package_info().version.to_string();
    let mut updater = webview
        .updater_builder()
        .endpoints(endpoints)?
        .header("Accept", "application/json")?
        .header("X-Axolotl-Channel", channel)?
        .header("X-Axolotl-Platform", platform)?
        .header("X-Axolotl-Version", current_version)?;

    #[cfg(target_os = "windows")]
    {
        let install_dir = std::env::current_exe()
            .map_err(|error| {
                theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                    "Failed to resolve current executable: {error}"
                )))
            })?
            .parent()
            .ok_or_else(|| {
                theseus::Error::from(theseus::ErrorKind::OtherError(
                    "Current executable has no parent directory".to_string(),
                ))
            })?
            .to_path_buf();

        tracing::debug!(
            install_dir = %install_dir.display(),
            "Using current executable directory for Windows app updates"
        );
        updater = updater.installer_arg(format!(
            "/INSTALL_DIR=\"{}\"",
            install_dir.display()
        ));
    }

    let updater = updater.build()?;
    updater.check().await.map_err(Into::into)
}

/// Check the updater manifest through the configured Update Server endpoint,
/// falling back to GitLink only when that endpoint is unusable.
async fn check_with_updater<R: Runtime>(
    webview: &Webview<R>,
    channel: &str,
) -> Result<Option<UpdateMetadata>> {
    // 未知通道是配置错误，保持既有错误语义，不去尝试降级源。
    let source = crate::updater_fallback::UpdateChannel::parse(channel)
        .ok_or_else(|| update_channel_error(channel))?;

    let started = Instant::now();
    let primary =
        check_with_endpoints(webview, channel, vec![update_endpoint()?]).await;

    match primary {
        Ok(Some(mut update)) => {
            let elapsed = started.elapsed();
            log_check_outcome(
                "update-server",
                Some(&update.version),
                elapsed,
                None,
            );

            // 主源成功但很慢时，仍然先把下载地址改到 GitLink 镜像：这只决定
            // 「下载开始前用哪个主机」，签名照旧由插件校验。快速路径不进入这个
            // 分支，因此正常检查不会多发一次 GitLink 列表请求。
            if crate::updater_fallback::is_slow_source(
                elapsed,
                UPDATE_MIRROR_LATENCY_THRESHOLD,
            ) {
                let tag =
                    crate::updater_fallback::tag_for_version(&update.version);
                // 列表拉取失败不算错误：保持原地址，主源结果不受影响。
                let releases = fetch_gitlink_releases().await;
                let mirrored = match releases.as_result() {
                    Ok(body) => {
                        crate::updater_fallback::mirrored_filenames(body, &tag)
                    }
                    Err(_) => Vec::new(),
                };
                let original = update.download_url.clone();
                apply_gitlink_mirror(&mut update, &tag, &mirrored);
                tracing::info!(
                    source = "update-server",
                    elapsed_ms = elapsed.as_millis(),
                    threshold_ms = UPDATE_MIRROR_LATENCY_THRESHOLD.as_millis(),
                    advised_tag = %tag,
                    mirror_hit = update.download_url != original,
                    "Slow update source, checked the GitLink mirror"
                );
            }

            build_update_metadata(webview, update).await
        }
        // 主源可用且明确表示没有更新。此时不查降级源：降级源的版本来自
        // tag 排序，不是 Update Server 认可过的最新版本，主动去找会误报到
        // 未经验证的版本。
        Ok(None) => {
            log_check_outcome("update-server", None, started.elapsed(), None);
            Ok(None)
        }
        Err(error) => {
            let reason = error.to_string();
            log_check_outcome(
                "update-server",
                None,
                started.elapsed(),
                Some(&reason),
            );

            let fallback_started = Instant::now();
            let fallback = check_with_fallback_endpoint(webview, source).await;
            match fallback {
                Some(update) => {
                    log_check_outcome(
                        "gitlink",
                        Some(&update.version),
                        fallback_started.elapsed(),
                        None,
                    );
                    build_update_metadata(webview, update).await
                }
                None => {
                    // 两条都失败时保留主源错误语义，由调用方按原有方式呈现。
                    log_check_outcome(
                        "gitlink",
                        None,
                        fallback_started.elapsed(),
                        Some("no usable release"),
                    );
                    Err(error)
                }
            }
        }
    }
}

/// 把降级清单里指向 GitHub 的下载地址改写成 GitLink 镜像地址。
///
/// 之所以可以改写：GitLink 镜像复刻的是同一条流水线的同一份字节，而 minisign 签名
/// 覆盖整个文件内容、与 URL 无关——只要字节一致，清单里的 `signature` 原值就仍能
/// 通过校验。因此这里只替换 `download_url`，不碰 `signature`，也不动 `timeout`、
/// `headers` 等其它字段。
///
/// 若镜像字节与签名目标不一致（同步了别的构建、文件被替换、传输被截断），下载完成
/// 后的签名校验会失败，插件放弃安装、不会装上任何东西。这是期望的安全失败：这里
/// 不放宽校验，也不做「签名失败就换回 GitHub 重试」的兜底。
///
/// 镜像缺该文件时保持清单原地址，行为与引入改写之前一致。
fn apply_gitlink_mirror(update: &mut Update, tag: &str, mirrored: &[String]) {
    // apt 分支稍后会用 Update Server 目录里的 deb 覆盖 `download_url`，并以
    // 那份目录自带的 sha256/size 校验为准；这里不介入那条路径。
    if is_apt_linux() {
        return;
    }

    let original = update.download_url.clone();
    match crate::updater_fallback::mirror_download_url_for(
        &original, tag, mirrored,
    ) {
        Some(mirrored_url) => {
            tracing::info!(
                source = "gitlink",
                tag,
                original = %original,
                mirrored = %mirrored_url,
                "Rewriting the update download to the GitLink mirror"
            );
            update.download_url = mirrored_url;
        }
        None => {
            tracing::debug!(
                source = "gitlink",
                tag,
                original = %original,
                "GitLink mirror does not provide this file, keeping the manifest url"
            );
        }
    }
}

/// 用 GitLink 推导出的端点再跑一次 updater 检查。
///
/// 任何失败都返回 `None`：降级源不可用不等于整个检查失败，主源错误已经
/// 由调用方保留。
async fn check_with_fallback_endpoint<R: Runtime>(
    webview: &Webview<R>,
    source: crate::updater_fallback::UpdateChannel,
) -> Option<Update> {
    let releases = fetch_gitlink_releases().await;
    let endpoint = crate::updater_fallback::endpoint_from_releases(
        releases.as_result(),
        source,
    )?;

    let manifest_url = endpoint.url.clone();
    if !crate::updater_fallback::probe_manifest(&manifest_url).await {
        tracing::warn!(
            source = "gitlink",
            tag = %endpoint.tag,
            url = %manifest_url,
            "GitLink latest.json is not a release manifest"
        );
        return None;
    }

    let channel = match source {
        crate::updater_fallback::UpdateChannel::Release => "release",
        crate::updater_fallback::UpdateChannel::Beta => "beta",
    };
    let fallback_check =
        check_with_endpoints(webview, channel, vec![endpoint.url]).await;

    match fallback_check {
        Ok(Some(mut update)) => {
            // 镜像清单 `platforms[*].url` 仍指向 github.com，只降级检查阶段的话
            // 下载还是会回到 GitHub，因此命中降级后把下载地址换到镜像。
            let mirrored = match releases.as_result() {
                Ok(body) => crate::updater_fallback::mirrored_filenames(
                    body,
                    &endpoint.tag,
                ),
                Err(_) => Vec::new(),
            };
            apply_gitlink_mirror(&mut update, &endpoint.tag, &mirrored);
            Some(update)
        }
        Ok(None) => {
            tracing::info!(
                source = "gitlink",
                tag = %endpoint.tag,
                "GitLink reports no newer version"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                source = "gitlink",
                tag = %endpoint.tag,
                failure = %error,
                "GitLink manifest is unusable"
            );
            None
        }
    }
}

/// 拉取 GitLink 的 release 列表。
async fn fetch_gitlink_releases() -> ReleaseList {
    match request_gitlink_releases().await {
        Ok(body) => ReleaseList::Fetched(body),
        Err(reason) => ReleaseList::Unavailable(reason),
    }
}

async fn request_gitlink_releases() -> std::result::Result<String, String> {
    let response = ClientBuilder::new()
        .user_agent(launcher_user_agent())
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|error| {
            format!("failed to build the GitLink client: {error}")
        })?
        .get(crate::updater_fallback::releases_url())
        .header(ACCEPT, HeaderValue::from_static("application/json"))
        .send()
        .await
        .map_err(|error| {
            format!("GitLink release list request failed: {error}")
        })?;

    if !response.status().is_success() {
        return Err(format!(
            "GitLink release list returned {}",
            response.status()
        ));
    }

    let body = response.text().await.map_err(|error| {
        format!("failed to read the GitLink release list: {error}")
    })?;

    // GitLink 用 HTTP 200 掩盖业务错误，状态码通过后仍要按响应体判定。
    if !crate::updater_fallback::looks_like_release_list(&body) {
        return Err(
            "GitLink release list body is not a release list".to_string()
        );
    }

    Ok(body)
}
/// GitLink release 列表的拉取结果；两种状态都自持数据，可跨 await 使用。
enum ReleaseList {
    Fetched(String),
    Unavailable(String),
}

impl ReleaseList {
    fn as_result(&self) -> std::result::Result<&str, &str> {
        match self {
            Self::Fetched(body) => Ok(body.as_str()),
            Self::Unavailable(reason) => Err(reason.as_str()),
        }
    }
}

/// 把插件给出的 `Update` 转成前端契约里的元数据。
async fn build_update_metadata<R: Runtime>(
    webview: &Webview<R>,
    mut update: Update,
) -> Result<Option<UpdateMetadata>> {
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);

    // On Debian and derivatives the plugin's minisign signature check cannot
    // validate the unsigned .deb, so point the download at the deb from the
    // Update Server catalog instead of the AppImage artifact. Its integrity
    // is verified with the catalog's sha256/size during the download.
    if is_apt_linux() {
        update.download_url = fetch_apt_deb_asset(&update.version).await?.url;
    }

    let published_at = update
        .raw_json
        .get("published_at")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let force_update = update
        .raw_json
        .get("force_update")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let metadata = UpdateMetadata {
        rid: webview.resources_table().add(update.clone()),
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: None,
        body: update.body.clone(),
        published_at,
        force_update,
        raw_json: update.raw_json,
    };

    Ok(Some(metadata))
}

// ── Tauri commands ───────────────────────────────────────────────

#[tauri::command]
pub async fn check_app_update<R: Runtime>(
    webview: Webview<R>,
    channel: String,
) -> Result<Option<UpdateMetadata>> {
    check_with_updater(&webview, &channel).await
}

// Reimplementation of Update::download mostly, minus the actual download part
#[tauri::command]
pub async fn get_update_size<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<Option<u64>> {
    let update = webview.resources_table().get::<Update>(rid)?;

    let mut headers = update.headers.clone();
    if !headers.contains_key(ACCEPT) {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/octet-stream"),
        );
    }

    let mut request = ClientBuilder::new().user_agent(launcher_user_agent());
    if let Some(timeout) = update.timeout {
        request = request.timeout(timeout);
    }
    if let Some(ref proxy) = update.proxy {
        let proxy = reqwest::Proxy::all(proxy.as_str())?;
        request = request.proxy(proxy);
    }
    let response = request
        .build()?
        .head(update.download_url.clone())
        .headers(headers)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Download request failed with status: {}",
            response.status()
        ))
        .into());
    }

    let content_length = response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());

    Ok(content_length)
}

#[tauri::command]
pub async fn enqueue_update_for_installation<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<()> {
    let pending_data = webview.state::<PendingUpdateData>().inner();

    let update = webview.resources_table().get::<Update>(rid)?;

    let progress = init_loading(
        LoadingBarType::LauncherUpdate {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
        },
        1.0,
        "Downloading update...",
    )
    .await?;

    let download_start = Instant::now();
    let update_data = if is_apt_linux() {
        // The .deb carries no minisign signature, so the plugin's signed
        // download cannot be used. Fetch the catalog entry and verify the
        // downloaded bytes against its sha256 and size instead.
        let asset = fetch_apt_deb_asset(&update.version).await?;

        let mut headers = update.headers.clone();
        if !headers.contains_key(ACCEPT) {
            headers.insert(
                ACCEPT,
                HeaderValue::from_static("application/octet-stream"),
            );
        }

        let mut request =
            ClientBuilder::new().user_agent(launcher_user_agent());
        if let Some(timeout) = update.timeout {
            request = request.timeout(timeout);
        }
        if let Some(ref proxy) = update.proxy {
            let proxy = reqwest::Proxy::all(proxy.as_str())?;
            request = request.proxy(proxy);
        }
        let response = request
            .build()?
            .get(update.download_url.clone())
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::Network(format!(
                "Download request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let total_size = response.content_length().unwrap_or(asset.size);
        let mut buffer = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.extend_from_slice(&chunk);
            if total_size > 0
                && let Err(e) = emit_loading(
                    &progress,
                    buffer.len() as f64 / total_size as f64,
                    None,
                )
            {
                tracing::error!("Failed to update download progress bar: {e}");
            }
        }

        if buffer.len() as u64 != asset.size {
            return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
                format!(
                    "Downloaded deb size mismatch: expected {}, got {}",
                    asset.size,
                    buffer.len()
                ),
            ))
            .into());
        }

        let digest = Sha256::digest(&buffer);
        let digest_hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest_hex != asset.sha256 {
            return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
                "Downloaded deb sha256 mismatch".to_string(),
            ))
            .into());
        }

        buffer
    } else {
        update
            .download(
                |chunk_size, total_size| {
                    let Some(total_size) = total_size else {
                        return;
                    };
                    if let Err(e) = emit_loading(
                        &progress,
                        chunk_size as f64 / total_size as f64,
                        None,
                    ) {
                        tracing::error!(
                            "Failed to update download progress bar: {e}"
                        );
                    }
                },
                || {},
            )
            .await?
    };
    let download_duration = download_start.elapsed();
    tracing::info!("Downloaded update in {download_duration:?}");

    pending_data
        .0
        .lock()
        .unwrap()
        .replace((update, update_data));

    Ok(())
}

#[tauri::command]
pub fn remove_enqueued_update<R: Runtime>(webview: Webview<R>) {
    let pending_data = webview.state::<PendingUpdateData>().inner();
    pending_data.0.lock().unwrap().take();
}

// ── Debian / derivatives apt update ─────────────────────────────

/// Whether this Linux system updates Axolotl through apt (Debian and its
/// derivatives) and has `pkexec` available for a single privileged prompt.
#[tauri::command]
pub fn is_apt_linux() -> bool {
    #[cfg(target_os = "linux")]
    {
        let debian_like = std::path::Path::new("/etc/debian_version").exists()
            || std::path::Path::new("/etc/apt").is_dir()
            || std::path::Path::new("/usr/bin/apt-get").exists();
        let has_pkexec = ["/usr/bin/pkexec", "/bin/pkexec"]
            .iter()
            .any(|path| std::path::Path::new(path).exists());
        debian_like && has_pkexec
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Install the downloaded .deb on Debian and its derivatives, prompting for
/// root once via `pkexec`. The package is installed from the absolute path
/// of a temporary file, which is removed afterwards.
pub async fn install_apt_package(version: &str, data: &[u8]) -> Result<()> {
    if !is_apt_linux() {
        return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            "apt updates are only supported on Debian-based Linux systems with pkexec"
                .to_string(),
        ))
        .into());
    }

    let arch = apt_deb_arch()?;
    let deb_path = std::env::temp_dir()
        .join(format!("Axolotl.Launcher_{version}_{arch}.deb"));
    std::fs::write(&deb_path, data).map_err(|io| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to write the downloaded deb: {io}"
        )))
    })?;
    let _deb_cleanup = TempDebFile(deb_path.clone());

    let install_path = deb_path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("pkexec")
            .arg("apt")
            .arg("install")
            .arg("-y")
            .arg(&install_path)
            .output()
    })
    .await
    .map_err(|join| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to run the apt updater: {join}"
        )))
    })?
    .map_err(|io| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to start pkexec: {io}"
        )))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            format!("apt install failed: {}", stderr.trim()),
        ))
        .into());
    }

    Ok(())
}

/// Removes a temporary Debian package when installation finishes or fails.
///
/// The installer awaits a blocking task and has several fallible operations
/// after creating the file. Keeping cleanup in `Drop` makes every return path
/// (including task and process-launch errors) remove the package.
struct TempDebFile(std::path::PathBuf);

impl Drop for TempDebFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.0.display(),
                error = %error,
                "Failed to remove temporary deb file"
            );
        }
    }
}
