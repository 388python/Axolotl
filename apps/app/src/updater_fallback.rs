//! GitLink 更新源降级实现。
//!
//! 主更新源 `update.axlmc.org/latest` 不可用时（超时 / 403 / 5xx / 响应体解析失败），
//! 本模块通过 GitLink 镜像推断最新 release，并把拼好的 `latest.json` 地址交给
//! updater 插件解析，从而复用插件既有的清单结构与 minisign 签名校验。
//!
//! GitLink 没有「按 channel 返回最新」的接口，因此降级分两步：
//! 1. `GET /api/{owner}/{repo}/releases.json` 取全量 release（公开仓库免鉴权）；
//! 2. 过滤草稿、按 tag 语义化排序、按 channel 选 tag，再拼
//!    `/releases/download/{tag}/latest.json` 交给 `Updater::check` 解析。
//!
//! 为什么不自己解析 `latest.json` 再构造 `Update`：
//! `tauri_plugin_updater::Update` 的 `extract_path`、`context` 是私有字段，
//! 本 crate 无法构造，而下载与安装又依赖插件内部状态（代理、Windows 安装参数、
//! minisign 公钥）。让插件去请求拼好的 GitLink 端点，`Update` 的创建方式与签名
//! 校验都保持插件原样。
//!
//! 为什么降级只发生在检查阶段、下载开始后不再换源：
//! `Update::download` 是一次性 GET，minisign 签名覆盖整个文件字节。下载中途换到
//! 另一台主机上的同名文件会破坏签名一致性与断点完整性（镜像同步延迟时两个源的
//! 字节甚至可能不是同一版本）。因此换源只在检查阶段决定一次，`download_url` 在
//! 下载开始前就已确定。下载速度慢不会触发任何换源。
//!
//! 检查阶段的耗时慢则另说：主源虽然成功但超过阈值时，调用方会用
//! `updater_impl::UPDATE_MIRROR_LATENCY_THRESHOLD` 判断，并借本模块的
//! `tag_for_version` / `mirrored_filenames` 把 `download_url` 改写到镜像。
//! 那只改变「下载开始前选哪个主机」，同样不涉及下载中途换源，也不放宽签名校验。
//!
//! 为什么可以把 `download_url` 改写成 GitLink 地址：
//! GitLink 镜像 `latest.json` 里的 `platforms[*].url` 仍指向 github.com，只降级
//! 检查阶段的话，真正下载还是会回到 GitHub，对受限网络等于没有降级。镜像复刻
//! release 时同步的是同一条流水线的同一份字节，而 minisign 签名覆盖整个文件内容、
//! 与 URL 无关：只要字节一致，清单里的 `signature` 原值就仍然能通过校验。所以改写
//! 时只替换 URL，不碰 `signature`，也不动 `timeout`、`headers` 等其它字段。
//! 若镜像字节与签名目标不一致（同步了别的构建、文件被替换、传输被截断），插件在
//! 下载完成后的签名校验会失败并放弃安装——这是期望的安全失败，本模块不放宽校验，
//! 也不做「签名失败就换回 GitHub 重试」的兜底。

use serde::Deserialize;
use tauri::http::HeaderValue;
use tauri::http::header::ACCEPT;
use tauri_plugin_http::reqwest::ClientBuilder;
use theseus::launcher_user_agent;
use url::Url;

/// 探测 `latest.json` 的请求超时；只影响探测本身，不参与降级判断。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// GitLink 站点根地址，release 列表接口与附件下载都挂在它下面。
const GITLINK_BASE: &str = "https://www.gitlink.org.cn";
/// GitLink 上的 Axolotl 镜像仓库。
const GITLINK_REPO: &str = "xinvxueyuan/Axolotl";

/// `draft` 字段表示草稿。GitLink 用本地化字符串而非布尔值表达该状态。
const GITLINK_DRAFT_VALUE: &str = "草稿";

/// 更新通道，取值与主更新源的 `X-Axolotl-Channel` 请求头一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateChannel {
    /// 仅稳定版。
    Release,
    /// 优先预发布版，没有预发布时回落到稳定版。
    Beta,
}

impl UpdateChannel {
    /// 解析通道名；未知通道名返回 `None`，由调用方按配置错误处理。
    pub fn parse(channel: &str) -> Option<Self> {
        match channel {
            "release" => Some(Self::Release),
            "beta" => Some(Self::Beta),
            _ => None,
        }
    }
}

/// `releases.json` 的顶层结构。
///
/// GitLink 用 `status` 表达业务结果：`0` 成功，非 `0` 失败。HTTP 200 不代表成功，
/// 因此不能只依赖状态码判断可用性。
#[derive(Deserialize)]
pub struct ReleasesResponse {
    status: i64,
    #[serde(default)]
    releases: Option<Vec<ReleaseEntry>>,
}

/// `releases.json` 中的单个 release。
///
/// 只声明降级需要的字段。GitLink 不返回 `prerelease`，预发布状态只能由 tag 推断。
#[derive(Deserialize)]
pub struct ReleaseEntry {
    tag_name: String,
    /// 取值为 "草稿"（草稿）或 "稳定"（已发布）等字符串。
    #[serde(default)]
    draft: Option<String>,
    /// release 附件；字段缺失或为 `null` 时视为没有附件，不报错。
    #[serde(default)]
    attachments: Option<Vec<Attachment>>,
}

/// `releases.json` 中的单个附件（GitHub release asset 的镜像表示）。
///
/// 只声明 `title`：GitLink 用它表示资产文件名，与下载地址 path 的最后一段一致，
/// 用它做精确匹配即可。`title` 缺失或为 `null` 时不报错，该附件被忽略。
#[derive(Deserialize)]
pub struct Attachment {
    #[serde(default)]
    title: Option<String>,
}

/// 从 `releases.json` 响应体挑选本次应当检查的 tag。
///
/// 失败返回描述原因的错误字符串，由调用方降级为「该源不可用」并记日志；
/// 这里不会 panic，也不会把错误抛给主源成功时的调用路径。
pub fn select_release_tag(
    body: &str,
    channel: UpdateChannel,
) -> Result<String, String> {
    let response: ReleasesResponse = serde_json::from_str(body)
        .map_err(|error| format!("release list is not valid JSON: {error}"))?;

    if response.status != 0 {
        return Err(format!(
            "release list reported status {}, HTTP 200 does not mean success",
            response.status
        ));
    }

    let releases = response
        .releases
        .ok_or_else(|| "release list has no releases array".to_string())?;

    select_tag_from_releases(&releases, channel)
}

/// 从已解析的 release 列表按 channel 选出 tag。
///
/// 排序只依赖 tag 本身：GitLink 的 `created_at` 是本地时间字符串且不带时区，
/// 不适合跨时区比较；镜像分批补齐时也不保证它是升序的。
fn select_tag_from_releases(
    releases: &[ReleaseEntry],
    channel: UpdateChannel,
) -> Result<String, String> {
    let mut stable: Vec<&str> = Vec::new();
    let mut prerelease: Vec<&str> = Vec::new();

    for release in releases {
        if release.draft.as_deref() == Some(GITLINK_DRAFT_VALUE) {
            continue;
        }

        let tag = release.tag_name.trim();
        if parse_tag(tag).is_none() {
            tracing::debug!(
                tag,
                "Skipping GitLink release with an unparsable tag"
            );
            continue;
        }

        if is_prerelease_tag(tag) {
            prerelease.push(tag);
        } else {
            stable.push(tag);
        }
    }

    stable.sort_by(|left, right| compare_tags(right, left));
    prerelease.sort_by(|left, right| compare_tags(right, left));

    // beta 通道允许回落到稳定版。GitLink 镜像由流水线逐个复刻已有 release，
    // 预发布标签可能尚未同步；若此时硬性要求 beta，beta 通道用户会在主源故障时
    // 完全收不到更新。回落取到的仍是清单里声明、签名可比对的稳定版，
    // 不比「停在旧版本」更差，因此这里选择回落。
    let selected = match channel {
        UpdateChannel::Release => stable.first(),
        UpdateChannel::Beta => prerelease.first().or_else(|| stable.first()),
    };

    selected.map(|tag| (*tag).to_string()).ok_or_else(|| {
        format!("no published release matches the {channel:?} channel")
    })
}

/// `releases.json` 的响应体是否真的是 release 列表。
///
/// GitLink 用 HTTP 200 掩盖业务错误（`{status:-1}`）与不存在的路由（SPA HTML），
/// 因此状态码通过后仍要按响应体判定。
pub fn looks_like_release_list(body: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(body),
        Ok(serde_json::Value::Object(map)) if map.contains_key("status")
    )
}

/// GitLink 上 `latest.json` 的地址；插件会把它当普通 Tauri 清单解析。
fn latest_json_url(tag: &str) -> String {
    format!("{GITLINK_BASE}/{GITLINK_REPO}/releases/download/{tag}/latest.json")
}

/// 某个 tag 在 GitLink 镜像上提供的附件文件名（`attachments[].title`）。
///
/// 解析失败、`status` 非 0、没有该 tag 都只返回空列表：调用方据此保留清单原地址，
/// 与「没有降级源」时的行为一致。`attachments` 与 `title` 缺失不会报错。
pub fn mirrored_filenames(body: &str, tag: &str) -> Vec<String> {
    let releases = match serde_json::from_str::<ReleasesResponse>(body) {
        Ok(response) if response.status == 0 => {
            response.releases.unwrap_or_default()
        }
        _ => return Vec::new(),
    };

    releases
        .iter()
        .find(|release| release.tag_name.trim() == tag)
        .map(|release| {
            release
                .attachments
                .iter()
                .flatten()
                .filter_map(|attachment| attachment.title.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// 清单声明的下载地址中的文件名：URL path 的最后一段，并做 percent-decode。
///
/// 清单里的 `url` 可能是 `.../Axolotl%20Launcher_1.10.0_x64-setup.nsis.zip` 这种
/// 百分号编码形式，而镜像附件 `title` 是原始文件名，所以必须先解码再匹配。
pub fn download_filename(url: &Url) -> Option<String> {
    let last = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    let decoded = urlencoding::decode(last).ok()?;
    if decoded.is_empty() {
        return None;
    }
    Some(decoded.into_owned())
}

/// 若镜像确实提供 `filename`，返回它在 `tag` 下的 GitLink 下载地址。
///
/// 只有 `title` 精确命中才改写；镜像缺该文件时返回 `None`，调用方保留清单原地址，
/// 行为与引入改写之前一致。
pub fn mirror_download_url(
    tag: &str,
    filename: &str,
    mirrored: &[String],
) -> Option<Url> {
    let tag = tag.trim();
    if tag.is_empty() || filename.is_empty() {
        return None;
    }
    if !mirrored.iter().any(|title| title == filename) {
        return None;
    }

    let mut url = Url::parse(&format!(
        "{GITLINK_BASE}/{GITLINK_REPO}/releases/download/"
    ))
    .ok()?;
    // 基址以 `/download/` 结尾，去掉末尾空段再 push，避免拼出 `download//tag`。
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend([tag, filename]);
    Some(url)
}

/// 组合版：从清单声明的下载地址推出镜像地址，供降级路径直接调用。
pub fn mirror_download_url_for(
    manifest_url: &Url,
    tag: &str,
    mirrored: &[String],
) -> Option<Url> {
    let filename = download_filename(manifest_url)?;
    mirror_download_url(tag, &filename, mirrored)
}

/// tag 是否表示预发布：形如 `v1.10.0-beta.5`、`v1.11.0-rc.1`。
pub fn is_prerelease_tag(tag: &str) -> bool {
    tag.contains('-')
}

/// 版本号主体：主版本、次版本、补丁版本。
type Version = (u64, u64, u64);

/// 预发布标识符：点分隔的段序列，按段比较。
type Prerelease = Vec<PrereleasePart>;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PrereleasePart {
    /// 纯数字段按数值比较，因此 `beta.10` 大于 `beta.9`。
    Numeric(u64),
    /// 文本段按字典序比较。
    Text(String),
}

/// 解析 `v1.10.0` / `1.10.0` / `v1.10.0-beta.5` / `v1.10.0-beta.5+build.7`。
///
/// 无法解析时返回 `None`，由调用方跳过该 release，避免排序时 panic。
fn parse_tag(tag: &str) -> Option<(Version, Prerelease)> {
    let tag = tag.trim();
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let tag = tag.split_once('+').map_or(tag, |(core, _)| core);

    let (core, prerelease) = match tag.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (tag, None),
    };

    let mut numbers = core.split('.').map(|part| part.parse::<u64>().ok());
    let major = numbers.next()??;
    let minor = numbers.next()??;
    let patch = match numbers.next() {
        Some(patch) => patch?,
        None => 0,
    };
    if numbers.next().is_some() {
        return None;
    }

    let prerelease = match prerelease {
        None => Prerelease::new(),
        Some(prerelease) => prerelease
            .split('.')
            .map(|part| {
                if part.is_empty() {
                    return None;
                }
                Some(match part.parse::<u64>() {
                    Ok(number) => PrereleasePart::Numeric(number),
                    Err(_) => PrereleasePart::Text(part.to_string()),
                })
            })
            .collect::<Option<Prerelease>>()?,
    };

    Some(((major, minor, patch), prerelease))
}

/// 比较两个 tag，排序语义等同 Cargo/semver。
///
/// 同一版本号下预发布小于稳定版（`v1.10.0-beta.5` 小于 `v1.10.0`），
/// 预发布之间按标识符逐段比较。任一方无法解析时退化为字符串比较，保证排序可用。
pub fn compare_tags(left: &str, right: &str) -> std::cmp::Ordering {
    match (parse_tag(left), parse_tag(right)) {
        (Some((left_version, left_pre)), Some((right_version, right_pre))) => {
            left_version.cmp(&right_version).then_with(|| {
                match (left_pre.is_empty(), right_pre.is_empty()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    (false, false) => left_pre.cmp(&right_pre),
                }
            })
        }
        _ => left.cmp(right),
    }
}

/// 版本号对应的 GitLink tag：`1.10.0` → `v1.10.0`。
///
/// 清单里的 `version` 不带 `v` 前缀（`1.10.0`、`1.10.0-beta.5`），GitLink 的 tag
/// 一律带前缀。已带前缀时原样返回，避免拼出 `vv1.10.0`；空串返回空串。
pub fn tag_for_version(version: &str) -> String {
    if version.is_empty() || version.starts_with('v') {
        return version.to_string();
    }
    format!("v{version}")
}

/// 主源耗时是否超过阈值：严格大于才算慢，正好等于阈值仍算正常。
///
/// 只决定「下载开始前用哪个主机」，不参与签名校验；阈值本身是经验值。
pub fn is_slow_source(
    elapsed: std::time::Duration,
    threshold: std::time::Duration,
) -> bool {
    elapsed > threshold
}

/// 降级源选中某个 release 后应构造的结果。
#[derive(Debug, PartialEq, Eq)]
pub struct FallbackEndpoint {
    /// 选中的 tag，用于日志与排障。
    pub tag: String,
    /// 交给 updater 插件请求的 `latest.json` 地址。
    pub url: Url,
}

/// 由 `releases.json` 响应体构造降级端点；任何失败都只返回 `None`。
///
/// 失败的源被当作「不可用」而不是错误：主源成功时它的结果不受影响，
/// 两条都失败时由调用方保留主源错误语义。
pub fn endpoint_from_releases(
    releases: Result<&str, &str>,
    channel: UpdateChannel,
) -> Option<FallbackEndpoint> {
    let tag = match releases {
        Ok(body) => match select_release_tag(body, channel) {
            Ok(tag) => tag,
            Err(reason) => {
                log_unavailable(&reason);
                return None;
            }
        },
        Err(reason) => {
            log_unavailable(reason);
            return None;
        }
    };

    let url = match Url::parse(&latest_json_url(&tag)) {
        Ok(url) => url,
        Err(error) => {
            log_unavailable(&format!(
                "unusable latest.json url for tag {tag}: {error}"
            ));
            return None;
        }
    };

    tracing::info!(
        source = "gitlink",
        %tag,
        %url,
        "Using the GitLink update source"
    );

    Some(FallbackEndpoint { tag, url })
}

/// GitLink release 列表接口地址，同样带 `unknown` 判定。
pub fn releases_url() -> String {
    format!("{GITLINK_BASE}/api/{GITLINK_REPO}/releases.json")
}

fn log_unavailable(reason: &str) {
    tracing::warn!(
        source = "gitlink",
        %reason,
        "GitLink update source is unavailable"
    );
}

/// 校验上游返回的 `latest.json` 文本是否真的是 Tauri 更新清单。
///
/// GitLink 用 HTTP 200 + HTML（或 `{status:-1}` 错误 JSON）表示资源不存在，
/// 因此必须检查响应体里是否有 `version` 字段，不能让状态码代替这一判定。
/// 真正的解析仍交给 updater 插件，这里只做「是不是清单」的前置判断。
pub fn looks_like_release_manifest(body: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(body),
        Ok(serde_json::Value::Object(map)) if map.contains_key("version")
    )
}

/// 在交给插件之前探测某个 tag 的 `latest.json` 是否真的是清单。
///
/// 镜像里并非每个 release 都带 `latest.json`，缺失时 GitLink 仍返回 HTTP 200。
/// 先探一次可以把「插件解析失败」提前成「该源不可用」，避免把 200 当成清单。
pub async fn probe_manifest(url: &url::Url) -> bool {
    let response = ClientBuilder::new()
        .user_agent(launcher_user_agent())
        .timeout(PROBE_TIMEOUT)
        .build();
    let client = match response {
        Ok(client) => client,
        Err(error) => {
            log_unavailable(&format!(
                "failed to build the probe client: {error}"
            ));
            return false;
        }
    };

    let body = match client
        .get(url.clone())
        .header(ACCEPT, HeaderValue::from_static("application/json"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            match response.text().await {
                Ok(body) => body,
                Err(error) => {
                    log_unavailable(&format!(
                        "failed to read the manifest: {error}"
                    ));
                    return false;
                }
            }
        }
        Ok(response) => {
            log_unavailable(&format!(
                "manifest request returned {}",
                response.status()
            ));
            return false;
        }
        Err(error) => {
            log_unavailable(&format!("manifest request failed: {error}"));
            return false;
        }
    };

    if !looks_like_release_manifest(&body) {
        log_unavailable("manifest body is not a release manifest");
        return false;
    }

    true
}
#[cfg(test)]
mod tests {
    use super::*;

    fn releases_body(status: i64, tags: &[(&str, &str)]) -> String {
        let entries: Vec<serde_json::Value> = tags
            .iter()
            .map(|(tag, draft)| {
                serde_json::json!({ "tag_name": tag, "draft": draft })
            })
            .collect();
        serde_json::json!({
            "status": status,
            "message": "ok",
            "releases": entries,
        })
        .to_string()
    }

    fn tag_of(body: &str, channel: UpdateChannel) -> String {
        select_release_tag(body, channel).expect("expected a selected tag")
    }

    #[test]
    fn sorts_tags_semantically_instead_of_lexically() {
        let mut tags = vec![
            "v1.9.9",
            "v1.10.0",
            "v1.10.0-beta.10",
            "v1.10.0-beta.2",
            "v1.10.0-rc.1",
        ];
        tags.sort_by(|left, right| compare_tags(right, left));
        assert_eq!(
            tags,
            vec![
                "v1.10.0",
                "v1.10.0-rc.1",
                "v1.10.0-beta.10",
                "v1.10.0-beta.2",
                "v1.9.9",
            ]
        );
    }

    #[test]
    fn stable_major_version_outranks_prerelease_of_same_version() {
        // 稳定版高于同版本的预发布：这是本降级链选 tag 的核心约束。
        assert!(
            compare_tags("v1.10.0", "v1.10.0-beta.5")
                == std::cmp::Ordering::Greater
        );
        assert!(
            compare_tags("v1.10.0-beta.5", "v1.10.0")
                == std::cmp::Ordering::Less
        );
        // 数值段按数值比较，不会出现 beta.10 < beta.9 的字符串排序错误。
        assert!(
            compare_tags("v1.10.0-beta.10", "v1.10.0-beta.9")
                == std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn release_channel_ignores_prereleases() {
        let body =
            releases_body(0, &[("v1.10.0-beta.5", "稳定"), ("v1.9.9", "稳定")]);
        assert_eq!(tag_of(&body, UpdateChannel::Release), "v1.9.9");
    }

    #[test]
    fn beta_channel_prefers_prerelease() {
        let body = releases_body(
            0,
            &[
                ("v1.10.0", "稳定"),
                ("v1.10.0-beta.5", "稳定"),
                ("v1.11.0-beta.1", "稳定"),
            ],
        );
        assert_eq!(tag_of(&body, UpdateChannel::Beta), "v1.11.0-beta.1");
    }

    #[test]
    fn beta_channel_falls_back_to_stable_when_no_prerelease_exists() {
        let body = releases_body(0, &[("v1.10.0", "稳定"), ("v1.9.9", "稳定")]);
        assert_eq!(tag_of(&body, UpdateChannel::Beta), "v1.10.0");
    }

    #[test]
    fn drafts_are_filtered_out() {
        let body =
            releases_body(0, &[("v1.11.0", "草稿"), ("v1.10.0", "稳定")]);
        assert_eq!(tag_of(&body, UpdateChannel::Release), "v1.10.0");

        let only_drafts = releases_body(0, &[("v1.11.0", "草稿")]);
        assert!(
            select_release_tag(&only_drafts, UpdateChannel::Release).is_err()
        );
    }

    #[test]
    fn non_zero_status_marks_the_source_unavailable() {
        // HTTP 200 + status:-1 是 GitLink 表达业务错误的方式。
        let body = releases_body(-1, &[("v1.10.0", "稳定")]);
        let reason =
            select_release_tag(&body, UpdateChannel::Release).unwrap_err();
        assert!(reason.contains("status -1"), "unexpected reason: {reason}");
        assert!(
            endpoint_from_releases(Ok(&body), UpdateChannel::Release).is_none()
        );
    }

    #[test]
    fn missing_releases_array_marks_the_source_unavailable() {
        let body = "{\"status\":0,\"message\":\"ok\"}";
        assert!(select_release_tag(body, UpdateChannel::Release).is_err());
    }

    #[test]
    fn transport_failure_marks_the_source_unavailable() {
        assert!(
            endpoint_from_releases(Err("timeout"), UpdateChannel::Release)
                .is_none()
        );
    }

    #[test]
    fn html_error_page_is_not_a_release_manifest() {
        // GitLink 对不存在的 latest.json 返回 HTTP 200 + HTML。
        assert!(!looks_like_release_manifest(
            "<!doctype html><html><body>Not Found</body></html>"
        ));
        assert!(!looks_like_release_manifest(
            "{\"status\":-1,\"message\":\"接口方法异常\"}"
        ));
        assert!(!looks_like_release_manifest(""));
        assert!(looks_like_release_manifest(
            "{\"version\":\"1.10.0\",\"platforms\":{}}"
        ));
    }

    #[test]
    fn endpoint_points_at_the_selected_tag_manifest() {
        let body = releases_body(0, &[("v1.10.0", "稳定")]);
        let endpoint =
            endpoint_from_releases(Ok(&body), UpdateChannel::Release)
                .expect("expected an endpoint");
        assert_eq!(endpoint.tag, "v1.10.0");
        assert_eq!(
            endpoint.url.as_str(),
            "https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/latest.json"
        );
    }

    #[test]
    fn channel_names_match_the_update_server_headers() {
        assert_eq!(
            UpdateChannel::parse("release"),
            Some(UpdateChannel::Release)
        );
        assert_eq!(UpdateChannel::parse("beta"), Some(UpdateChannel::Beta));
        assert_eq!(UpdateChannel::parse("nightly"), None);
    }

    fn mirror_titles(titles: &[&str]) -> Vec<String> {
        titles.iter().map(|title| (*title).to_string()).collect()
    }

    #[test]
    fn mirror_rewrites_the_download_url_to_gitlink() {
        // 镜像提供该文件时，下载地址改写到 GitLink，签名由调用方保持原值。
        let mirrored = mirror_titles(&[
            "Axolotl.Launcher_1.10.0_x64-setup.nsis.zip",
            "Axolotl.Launcher_1.10.0_x64-setup.nsis.zip.sig",
        ]);
        let url = mirror_download_url(
            "v1.10.0",
            "Axolotl.Launcher_1.10.0_x64-setup.nsis.zip",
            &mirrored,
        )
        .expect("the mirror provides this file");
        assert_eq!(
            url.as_str(),
            "https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_x64-setup.nsis.zip"
        );
    }

    #[test]
    fn mirror_is_skipped_when_the_mirror_lacks_the_file() {
        // 镜像缺文件时必须返回 None，调用方保留清单原地址。
        let mirrored =
            mirror_titles(&["Axolotl.Launcher_1.10.0_amd64.AppImage"]);
        assert!(
            mirror_download_url(
                "v1.10.0",
                "Axolotl.Launcher_1.10.0_x64-setup.nsis.zip",
                &mirrored,
            )
            .is_none()
        );
        // 标题是精确匹配：前缀相同但不是同一个文件也不能命中。
        assert!(
            mirror_download_url(
                "v1.10.0",
                "Axolotl.Launcher_1.10.0_x64-setup.nsis",
                &mirrored,
            )
            .is_none()
        );
        assert!(mirror_download_url("v1.10.0", "anything.zip", &[]).is_none());
    }

    #[test]
    fn percent_encoded_manifest_filename_matches_the_mirror() {
        // 清单 url 可能是百分号编码，镜像 title 是原始文件名：先 decode 再匹配。
        let manifest = Url::parse(
            "https://github.com/Axolotl-Launcher/Axolotl/releases/download/v1.10.0/Axolotl%20Launcher_1.10.0_x64-setup.nsis.zip",
        )
        .expect("valid manifest url");
        assert_eq!(
            download_filename(&manifest).as_deref(),
            Some("Axolotl Launcher_1.10.0_x64-setup.nsis.zip")
        );

        let mirrored =
            mirror_titles(&["Axolotl Launcher_1.10.0_x64-setup.nsis.zip"]);
        let mirrored_url =
            mirror_download_url_for(&manifest, "v1.10.0", &mirrored)
                .expect("decoded filename should match the attachment title");
        assert_eq!(
            mirrored_url.as_str(),
            "https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl%20Launcher_1.10.0_x64-setup.nsis.zip"
        );
    }

    #[test]
    fn mirrored_filenames_come_from_attachments_of_the_matching_tag() {
        let body = serde_json::json!({
            "status": 0,
            "releases": [
                {
                    "tag_name": "v1.10.0",
                    "draft": "稳定",
                    "attachments": [
                        {
                            "id": 1,
                            "title": "Axolotl.Launcher_1.10.0_x64-setup.nsis.zip",
                        },
                        { "id": 2 },
                        { "title": null },
                    ],
                },
                {
                    "tag_name": "v1.9.9",
                    "draft": "稳定",
                    "attachments": [
                        { "id": 3, "title": "Axolotl.Launcher_1.9.9_x64-setup.nsis.zip" },
                    ],
                },
                { "tag_name": "v1.8.0", "draft": "稳定" },
            ],
        })
        .to_string();

        assert_eq!(
            mirrored_filenames(&body, "v1.10.0"),
            vec!["Axolotl.Launcher_1.10.0_x64-setup.nsis.zip".to_string()]
        );
        // 缺 attachments 的 tag 不是错误，只是没有可镜像的文件。
        assert!(mirrored_filenames(&body, "v1.8.0").is_empty());
        assert!(mirrored_filenames(&body, "v2.0.0").is_empty());
        assert!(mirrored_filenames("not json", "v1.10.0").is_empty());
    }

    #[test]
    fn tag_for_version_adds_the_missing_v_prefix() {
        assert_eq!(tag_for_version("1.10.0"), "v1.10.0");
        // 预发布标识符不参与前缀拼接。
        assert_eq!(tag_for_version("1.10.0-beta.5"), "v1.10.0-beta.5");
    }

    #[test]
    fn tag_for_version_keeps_an_existing_v_prefix() {
        // 已经带前缀时必须原样返回，不能拼出 vv1.10.0。
        assert_eq!(tag_for_version("v1.10.0"), "v1.10.0");
        assert_eq!(tag_for_version("v1.10.0-beta.5"), "v1.10.0-beta.5");
        // 空串没有可拼的版本号。
        assert_eq!(tag_for_version(""), "");
    }

    #[test]
    fn slow_source_requires_strictly_more_than_the_threshold() {
        let threshold = std::time::Duration::from_secs(3);
        assert!(!is_slow_source(
            std::time::Duration::from_secs(2),
            threshold
        ));
        // 正好等于阈值不算慢，只有严格大于才换源。
        assert!(!is_slow_source(threshold, threshold));
        assert!(is_slow_source(
            threshold + std::time::Duration::from_millis(1),
            threshold
        ));
    }
}
