import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
    getDownloadAvailability,
    getReleasePage,
    hasLauncherDownloads,
    resolveLauncherDownloads,
} from './launcher-downloads.ts'

test('a successful response without a platform package is unavailable, not still loading', () => {
    const links = resolveLauncherDownloads(
        { tag_name: 'v1.0-beta', assets: [{ name: 'Axolotl_universal.dmg' }] },
        'update-server',
    )
    assert.equal(getDownloadAvailability('success', links.windows), 'unavailable')
    assert.equal(getDownloadAvailability('success', links.mac), 'ready')
    assert.equal(getDownloadAvailability('success', links.appImage), 'unavailable')
    assert.equal(getDownloadAvailability('pending', links.mac), 'loading')
    assert.equal(getDownloadAvailability('error', links.mac), 'error')
    assert.equal(
        getDownloadAvailability(
            'success',
            resolveLauncherDownloads({ tag_name: 'v1', assets: [] }, 'github').windows,
        ),
        'unavailable',
    )
})

test('download links use only the supplied release and source, and fallback respects the channel', () => {
    const release = {
        tag_name: 'v2.0-beta.1',
        assets: [
            { name: 'Axolotl_x64-setup.exe', sha256: 'a'.repeat(64) },
            { name: 'Axolotl_amd64.AppImage', sha256: 'b'.repeat(64) },
            { name: 'Axolotl_amd64.deb', sha256: 'c'.repeat(64) },
        ],
    }
    assert.equal(
        resolveLauncherDownloads(release, 'github').windows,
        'https://github.com/Mystic-Stars/Axolotl/releases/download/v2.0-beta.1/Axolotl_x64-setup.exe',
    )
    assert.equal(resolveLauncherDownloads(null, 'github').windows, null)
    assert.equal(
        resolveLauncherDownloads(release, 'update-server').deb,
        'https://update.axlmc.org/dist/2.0-beta.1/Axolotl_amd64.deb',
    )
    assert.equal(
        resolveLauncherDownloads(release, 'update-server').appImage,
        'https://update.axlmc.org/dist/2.0-beta.1/Axolotl_amd64.AppImage',
    )
    assert.equal(
        getReleasePage('beta', release),
        'https://github.com/Mystic-Stars/Axolotl/releases/tag/v2.0-beta.1',
    )
    assert.equal(getReleasePage('beta'), 'https://github.com/Mystic-Stars/Axolotl/releases')
    assert.equal(
        getReleasePage('release'),
        'https://github.com/Mystic-Stars/Axolotl/releases/latest',
    )
})

test('the gitlink source downloads through the mirrored attachment urls', () => {
    const release = {
        tag_name: 'v2.0',
        assets: [
            {
                name: 'Axolotl_2.0_x64_nsis-setup.exe',
                url: '/xinvxueyuan/Axolotl/releases/download/v2.0/Axolotl_2.0_x64_nsis-setup.exe',
            },
            {
                name: 'Axolotl_2.0_amd64.deb',
                url: 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v2.0/Axolotl_2.0_amd64.deb',
            },
        ],
    }
    const links = resolveLauncherDownloads(release, 'gitlink')
    assert.equal(
        links.windows,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v2.0/Axolotl_2.0_x64_nsis-setup.exe',
    )
    assert.equal(
        links.deb,
        'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download/v2.0/Axolotl_2.0_amd64.deb',
    )
    // A release without mirrored urls cannot produce gitlink links.
    assert.equal(
        resolveLauncherDownloads(
            { tag_name: 'v2.0', assets: [{ name: 'Axolotl_2.0_amd64.deb' }] },
            'gitlink',
        ).deb,
        null,
    )
    assert(hasLauncherDownloads(release))
})
