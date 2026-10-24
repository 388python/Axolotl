import assert from 'node:assert/strict'
import { test } from 'node:test'

import { getGitlinkReleasesUrl, selectLauncherRelease } from './gitlink-releases.ts'
import {
    getReleasePage,
    hasLauncherDownloads,
    type LauncherRelease,
    resolveLauncherAsset,
    resolveLauncherDownloads,
    resolveWithFallback,
} from './launcher-downloads.ts'

const updateServerRelease: LauncherRelease = {
    tag_name: 'v1.10.0',
    assets: [
        { name: 'Axolotl_Launcher_1.10.0_x64_nsis-setup.exe' },
        { name: 'Axolotl.Launcher_1.10.0_universal.dmg' },
        { name: 'Axolotl.Launcher_1.10.0_amd64.AppImage' },
        { name: 'Axolotl.Launcher_1.10.0_amd64.deb' },
        { name: 'Axolotl.Launcher-1.10.0-1.x86_64.rpm' },
    ],
}

// 与 releases-gitlink 归一化后的形状一致：name 来自 attachments[].title，url 已拼成绝对地址。
const gitlinkRelease: LauncherRelease = {
    tag_name: 'v1.10.0',
    assets: [
        {
            name: 'Axolotl_Launcher_1.10.0_x64_nsis-setup.exe',
            url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl_Launcher_1.10.0_x64_nsis-setup.exe',
        },
        {
            name: 'Axolotl.Launcher_1.10.0_universal.dmg',
            url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_universal.dmg',
        },
        {
            name: 'Axolotl.Launcher_1.10.0_amd64.AppImage',
            url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_amd64.AppImage',
        },
        {
            name: 'Axolotl.Launcher_1.10.0_amd64.deb',
            url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_amd64.deb',
        },
        {
            name: 'Axolotl.Launcher-1.10.0-1.x86_64.rpm',
            url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher-1.10.0-1.x86_64.rpm',
        },
    ],
}

test('the primary source is used when it has usable assets', () => {
    const selection = selectLauncherRelease(
        { release: updateServerRelease },
        { release: gitlinkRelease },
        hasLauncherDownloads,
    )
    assert.equal(selection.source, 'update-server')
    assert.equal(
        selection.release,
        updateServerRelease,
        'the primary release must not be replaced by the fallback',
    )
    assert.equal(resolveWithFallback(updateServerRelease, gitlinkRelease), updateServerRelease)
})

test('a failed primary source falls back to GitLink without throwing', () => {
    const selection = selectLauncherRelease(
        { error: new Error('update-server timed out') },
        { release: gitlinkRelease },
        hasLauncherDownloads,
    )
    assert.equal(selection.source, 'gitlink')
    assert.equal(selection.release, gitlinkRelease)
    assert.equal(resolveWithFallback(null, gitlinkRelease), gitlinkRelease)
})

test('an empty or asset-less primary response falls back to GitLink', () => {
    for (const primary of [
        null,
        undefined,
        {},
        { release: null },
        { release: { tag_name: 'v1.10.0', assets: [] } },
    ]) {
        const selection = selectLauncherRelease(
            primary,
            { release: gitlinkRelease },
            hasLauncherDownloads,
        )
        assert.equal(
            selection.source,
            'gitlink',
            `primary ${JSON.stringify(primary)} must fall back`,
        )
    }
})

test('the underscore Windows installer naming counts as a usable download', () => {
    const underscoreNaming: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [{ name: 'Axolotl_Launcher_1.10.0_x64_nsis-setup.exe' }],
    }
    const dashNaming: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [{ name: 'Axolotl.Launcher_1.10.0_x64-setup.exe' }],
    }
    assert.equal(hasLauncherDownloads(underscoreNaming), true)
    assert.equal(hasLauncherDownloads(dashNaming), true)
})

test('a response carrying only signature files is not treated as usable', () => {
    const signatureOnly: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [{ name: 'Axolotl.Launcher_1.10.0_amd64.deb.sig' }],
    }
    assert.equal(hasLauncherDownloads(signatureOnly), false)
    const selection = selectLauncherRelease(
        { release: signatureOnly },
        { release: gitlinkRelease },
        hasLauncherDownloads,
    )
    assert.equal(selection.source, 'gitlink')
})

test('when both sources are unusable the selection stays empty and keeps the primary shape', () => {
    const selection = selectLauncherRelease(
        { release: null, error: new Error('down') },
        { release: { tag_name: 'v1.10.0', assets: [] } },
        hasLauncherDownloads,
    )
    assert.equal(selection.source, null)
    assert.equal(selection.release, null)
    assert.equal(resolveWithFallback(null, null), null)
})

test('gitlink assets match platform patterns and use the mirrored absolute URL', () => {
    const links = resolveLauncherDownloads(gitlinkRelease, 'gitlink')
    assert.equal(
        links.windows,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl_Launcher_1.10.0_x64_nsis-setup.exe',
    )
    assert.equal(
        links.mac,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_universal.dmg',
    )
    assert.equal(
        links.appImage,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_amd64.AppImage',
    )
    assert.equal(
        links.deb,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_amd64.deb',
    )
    assert.equal(
        links.rpm,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher-1.10.0-1.x86_64.rpm',
    )
})

test('a relative gitlink path is resolved against the GitLink origin', () => {
    const release: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [
            {
                name: 'Axolotl.Launcher_1.10.0_universal.dmg',
                url: '/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_universal.dmg',
            },
        ],
    }
    assert.equal(
        resolveLauncherAsset(release, 'Axolotl.Launcher_1.10.0_universal.dmg'),
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v1.10.0/Axolotl.Launcher_1.10.0_universal.dmg',
    )
    assert.equal(
        resolveLauncherDownloads(release, 'gitlink').mac,
        resolveLauncherAsset(release, 'Axolotl.Launcher_1.10.0_universal.dmg'),
    )
})

test('gitlink downloads never fall back to a guessed URL without asset metadata', () => {
    const release: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [{ name: 'Axolotl.Launcher_1.10.0_universal.dmg' }],
    }
    assert.equal(resolveLauncherDownloads(release, 'gitlink').mac, null)
    const unsafe: LauncherRelease = {
        tag_name: 'v1.10.0',
        assets: [{ name: 'Axolotl.Launcher_1.10.0_universal.dmg', url: 'javascript:alert(1)' }],
    }
    assert.equal(resolveLauncherDownloads(unsafe, 'gitlink').mac, null)
})

test('the gitlink request URL keeps the channel so SSR and CSR keys match', () => {
    assert.equal(getGitlinkReleasesUrl('release'), '/api/releases/gitlink?channel=release')
    assert.equal(getGitlinkReleasesUrl('beta'), '/api/releases/gitlink?channel=beta')
    assert.notEqual(getGitlinkReleasesUrl('release'), getGitlinkReleasesUrl('beta'))
})

test('a gitlink fallback still links to the matching GitHub release page', () => {
    assert.equal(
        getReleasePage('release', gitlinkRelease),
        'https://github.com/Mystic-Stars/Axolotl/releases/tag/v1.10.0',
    )
})
