import type { LauncherRelease, ReleaseChannel } from './launcher-downloads'

// GitLink 没有 CORS，浏览器不能直连；本站 Netlify 函数代为拉取并归一化。
// 该地址是构建期常量，与 useFetch 的 key 一同在 SSR/CSR 之间保持稳定。
export const GITLINK_RELEASES_PATH = '/api/releases/gitlink'

export const GITLINK_RELEASES_URL = `${GITLINK_RELEASES_PATH}?channel=release`

export function getGitlinkReleasesUrl(channel: ReleaseChannel) {
    return `${GITLINK_RELEASES_PATH}?channel=${channel}`
}

export interface SourceFetchResult {
    release?: LauncherRelease | null
    // NUXT_ERROR 表示本次请求失败（网络、超时、5xx 等），用于判断是否需要走降级源。
    error?: unknown
}

export type LauncherReleaseSource = 'update-server' | 'gitlink'

export interface LauncherReleaseSelection {
    release: LauncherRelease | null
    source: LauncherReleaseSource | null
}

/**
 * 在主源与降级源之间选择：主源请求失败或没有可用制品时改用降级源。
 * 这里只做判定，不发请求，也不抛异常；两者都不可用时返回空选择。
 * @param isUsable 判断一个发布是否带有可用制品，默认要求至少一个平台入口。
 */
export function selectLauncherRelease(
    primary: SourceFetchResult | null | undefined,
    fallback: SourceFetchResult | null | undefined,
    isUsable: (release: LauncherRelease | null | undefined) => boolean,
): LauncherReleaseSelection {
    const primaryRelease = primary?.error ? null : primary?.release
    if (primaryRelease && isUsable(primaryRelease)) {
        return { release: primaryRelease, source: 'update-server' }
    }
    const fallbackRelease = fallback?.error ? null : fallback?.release
    if (fallbackRelease && isUsable(fallbackRelease)) {
        return { release: fallbackRelease, source: 'gitlink' }
    }
    // 保留主源的数据形状（可能为 null），让现有展示逻辑继续使用版本号等字段。
    return { release: primaryRelease ?? null, source: null }
}
