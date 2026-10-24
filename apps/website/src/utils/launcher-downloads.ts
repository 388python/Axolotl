export type ReleaseChannel = 'release' | 'beta'
export type DownloadStatus = 'idle' | 'pending' | 'success' | 'error'
export interface ReleaseAsset {
    name: string
    sha256?: string
    /** 镜像源（GitLink）直接给出的制品地址；update-server 与 GitHub 由文件名拼接。 */
    url?: string
}

export interface LauncherRelease {
    tag_name: string
    assets: ReleaseAsset[]
}

// 与 use-download-source.ts 的 ResolvedDownloadSource 保持一致：'auto' 由调用方先行解析。
export type LauncherDownloadSource = 'update-server' | 'github' | 'gitlink'

const GITLINK_ORIGIN = 'https://www.gitlink.org.cn'

/**
 * 已解析的制品下载地址。仅 gitlink 提供，其他来源仍由 resolveLauncherDownloads 拼接规则地址。
 * @see resolveLauncherAsset
 */
export function resolveLauncherAsset(
    release: LauncherRelease | null | undefined,
    assetName: string,
): string | null {
    const asset = release?.assets.find((candidate) => candidate.name === assetName)
    if (!asset?.url || /^(data|javascript):/i.test(asset.url)) return null
    if (/^https?:\/\//i.test(asset.url)) return asset.url
    const path = asset.url.startsWith('/') ? asset.url : `/${asset.url}`
    return new URL(path, GITLINK_ORIGIN).toString()
}

/**
 * 判断一个发布是否带有可用于下载的制品，用于在主源与降级源之间做选择。
 * 只匹配固定平台的下载入口，避免只拿到签名文件也算作可用。
 * Windows 同时存在 `x64-setup.exe`（连字符）与 `x64_nsis-setup.exe`（下划线）两代命名，
 * 只认连字符形式会把 GitLink 镜像误判为缺少 Windows 包。
 */
export function hasLauncherDownloads(release: LauncherRelease | null | undefined) {
    if (!release || !Array.isArray(release.assets)) return false
    const patterns = [
        /x64-setup\.exe$/i,
        /x64_nsis-setup\.exe$/i,
        /universal\.dmg$/i,
        /(amd64|x86_64)\.AppImage$/i,
        /_amd64\.deb$/i,
        /(amd64|x86_64).*\.deb$/i,
        /(x86_64|amd64).*\.rpm$/i,
    ]
    return release.assets.some((asset) => patterns.some((pattern) => pattern.test(asset.name)))
}

/**
 * 在主源不可用时回退到降级源：主源为空、请求失败或没有可用制品都算不可用。
 * 两者都不可用时返回 null，由调用方决定展示什么。
 */
export function resolveWithFallback(
    primary: LauncherRelease | null | undefined,
    fallback: LauncherRelease | null | undefined,
) {
    return hasLauncherDownloads(primary)
        ? primary
        : hasLauncherDownloads(fallback)
          ? fallback
          : null
}

export function resolveLauncherDownloads(
    release: LauncherRelease | null | undefined,
    source: LauncherDownloadSource,
) {
    function findAsset(patterns: RegExp[]) {
        if (!release) return null
        const asset = patterns
            .map((pattern) => release.assets.find((candidate) => pattern.test(candidate.name)))
            .find(Boolean)
        if (!asset) return null
        // gitlink 的镜像路径与 GitHub 不同，必须用它自己返回的地址，不能按 tag 拼接。
        if (source === 'gitlink') return resolveLauncherAsset(release, asset.name)
        const filename = asset.name
        const tag = encodeURIComponent(release.tag_name)
        const version = encodeURIComponent(release.tag_name.replace(/^v/, ''))
        return source === 'update-server'
            ? `https://update.axlmc.org/dist/${version}/${encodeURIComponent(filename)}`
            : `https://github.com/Mystic-Stars/Axolotl/releases/download/${tag}/${encodeURIComponent(filename)}`
    }

    return {
        windows: findAsset([/x64-setup\.exe$/i, /\.exe$/i]),
        mac: findAsset([/universal\.dmg$/i, /\.dmg$/i]),
        appImage: findAsset([/(amd64|x86_64)\.AppImage$/i]),
        deb: findAsset([/_amd64\.deb$/i, /(amd64|x86_64).*\.deb$/i]),
        rpm: findAsset([/(x86_64|amd64).*\.rpm$/i]),
    }
}

export function getDownloadAvailability(status: DownloadStatus, link: string | null) {
    if (status === 'error') return 'error'
    if (status !== 'success') return 'loading'
    return link ? 'ready' : 'unavailable'
}

export function getReleasePage(channel: ReleaseChannel, release?: LauncherRelease | null) {
    const base = 'https://github.com/Mystic-Stars/Axolotl/releases'
    if (release?.tag_name) return `${base}/tag/${encodeURIComponent(release.tag_name)}`
    return channel === 'release' ? `${base}/latest` : base
}
