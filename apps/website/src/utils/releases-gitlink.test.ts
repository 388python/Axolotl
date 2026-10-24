import assert from 'node:assert/strict'
import { test } from 'node:test'

import handler from '../../../../netlify/functions/releases-gitlink.mjs'

const GITLINK_BASE = 'https://www.gitlink.org.cn/xinvxueyuan/Axolotl/releases/download'

function attachment(tag, title) {
    return {
        id: 1,
        title,
        filesize: '424 字节',
        description: '',
        is_pdf: false,
        url: `/xinvxueyuan/Axolotl/releases/download/${tag}/${title}`,
    }
}

function release(tag, draft, titles) {
    return {
        version_id: 1,
        id: '143789436',
        tag_name: tag,
        name: `Axolotl Launcher ${tag}`,
        body: 'notes',
        draft,
        created_at: '2026-10-24 04:24',
        attachments: titles.map((title) => attachment(tag, title)),
    }
}

const STABLE_ASSETS = [
    'Axolotl_Launcher_1.10.0_x64_nsis-setup.exe',
    'Axolotl.Launcher_1.10.0_universal.dmg',
    'Axolotl.Launcher_1.10.0_amd64.AppImage',
    'Axolotl.Launcher_1.10.0_amd64.deb',
    'Axolotl.Launcher-1.10.0-1.x86_64.rpm',
]

// GitLink 的业务错误也返回 HTTP 200，这里按真实形态构造响应。
function gitlinkResponse(
    body,
    { status = 200, contentType = 'application/json; charset=utf-8' } = {},
) {
    return new Response(typeof body === 'string' ? body : JSON.stringify(body), {
        status,
        headers: { 'Content-Type': contentType },
    })
}

function requestFor(channel) {
    const suffix = channel === undefined ? '' : `?channel=${channel}`
    return new Request(`https://axlmc.org/api/releases/gitlink${suffix}`)
}

async function callHandler({ payload, channel, onFetch }) {
    const originalFetch = globalThis.fetch
    const calls = []
    globalThis.fetch = async (url, init) => {
        calls.push({ url: String(url), init })
        return onFetch ? onFetch(url, init) : gitlinkResponse(payload)
    }
    try {
        const response = await handler(requestFor(channel))
        const text = await response.text()
        return { response, text, calls }
    } finally {
        globalThis.fetch = originalFetch
    }
}

test('the stable channel returns the newest stable tag with mirrored asset URLs', async () => {
    const payload = {
        status: 0,
        message: '响应成功',
        releases: [
            release('v1.10.0-beta.5', '稳定', [
                'Axolotl_Launcher_1.10.0-beta.5_x64_nsis-setup.exe',
            ]),
            release('v1.9.9', '稳定', ['Axolotl_Launcher_1.9.9_x64_nsis-setup.exe']),
            release('v1.10.0', '稳定', STABLE_ASSETS),
        ],
        total_count: 3,
    }
    const { response, text, calls } = await callHandler({ payload, channel: 'release' })
    assert.equal(response.status, 200)
    assert.equal(response.headers.get('cache-control'), 'public, max-age=0, s-maxage=300')
    const data = JSON.parse(text)
    assert.equal(data.tag_name, 'v1.10.0')
    assert.equal(data.assets.length, 5)
    assert.deepEqual(data.assets[0], {
        name: 'Axolotl_Launcher_1.10.0_x64_nsis-setup.exe',
        url: `${GITLINK_BASE}/v1.10.0/Axolotl_Launcher_1.10.0_x64_nsis-setup.exe`,
    })
    assert.equal(calls.length, 1)
    assert.equal(calls[0].url, 'https://www.gitlink.org.cn/api/xinvxueyuan/Axolotl/releases.json')
    assert.equal(calls[0].init.headers.Accept, 'application/json')
})

test('the beta channel selects prerelease tags and never a stable one', async () => {
    const payload = {
        status: 0,
        releases: [
            release('v1.10.0', '稳定', STABLE_ASSETS),
            release('v1.10.0-beta.5', '稳定', [
                'Axolotl_Launcher_1.10.0-beta.5_x64_nsis-setup.exe',
            ]),
            release('v1.10.0-beta.4', '稳定', [
                'Axolotl_Launcher_1.10.0-beta.4_x64_nsis-setup.exe',
            ]),
        ],
    }
    const { response, text } = await callHandler({ payload, channel: 'beta' })
    assert.equal(response.status, 200)
    const data = JSON.parse(text)
    assert.equal(data.tag_name, 'v1.10.0-beta.5')
    assert.equal(data.assets.length, 1)
})

test('tags are ordered semantically instead of by position or created_at', async () => {
    const payload = {
        status: 0,
        releases: [
            release('v1.9.9', '稳定', ['Axolotl_Launcher_1.9.9_x64_nsis-setup.exe']),
            release('v1.10.0', '稳定', STABLE_ASSETS),
            release('v1.8.0', '稳定', ['Axolotl_Launcher_1.8.0_x64_nsis-setup.exe']),
        ],
    }
    const { text } = await callHandler({ payload, channel: 'release' })
    assert.equal(JSON.parse(text).tag_name, 'v1.10.0')
})

test('draft releases are filtered out before the channel is selected', async () => {
    const payload = {
        status: 0,
        releases: [
            release('v1.11.0', '草稿', ['Axolotl_Launcher_1.11.0_x64_nsis-setup.exe']),
            release('v1.10.0', '稳定', STABLE_ASSETS),
        ],
    }
    const { text } = await callHandler({ payload, channel: 'release' })
    assert.equal(JSON.parse(text).tag_name, 'v1.10.0')
})

test('a missing channel match fails with 502 and is not cached', async () => {
    const payload = { status: 0, releases: [release('v1.10.0', '稳定', STABLE_ASSETS)] }
    const { response, text } = await callHandler({ payload, channel: 'beta' })
    assert.equal(response.status, 502)
    assert.equal(response.headers.get('cache-control'), null)
    assert.match(text, /No beta release on GitLink/)
})

test('HTTP 200 bodies that are not a GitLink success payload are rejected', async () => {
    const cases = [
        // 不存在的资源返回 200 + 错误 JSON
        { status: -1, message: '接口方法异常' },
        // 不存在的版本返回 200 + SPA HTML
        '<!DOCTYPE html><html><body>app</body></html>',
        { status: 0, releases: null },
        { status: '0', releases: [] },
        { status: 0, releases: [release('v1.10.0', '稳定', STABLE_ASSETS)] },
    ]
    for (const payload of cases) {
        const { response } = await callHandler({ payload, channel: 'beta' })
        assert.equal(response.status, 502, `payload ${JSON.stringify(payload)} must fail`)
        assert.equal(response.headers.get('cache-control'), null)
    }
    const empty = await callHandler({ payload: { status: 0, releases: [] }, channel: 'release' })
    assert.equal(empty.response.status, 502)
})

test('an invalid channel is rejected without contacting GitLink', async () => {
    const { response, calls } = await callHandler({
        payload: { status: 0, releases: [] },
        channel: 'nightly',
    })
    assert.equal(response.status, 400)
    assert.equal(response.headers.get('cache-control'), null)
    assert.equal(calls.length, 0)
})

test('an upstream failure or timeout becomes an uncached 502', async () => {
    const notOk = await callHandler({
        channel: 'release',
        onFetch: () => new Response('upstream down', { status: 503 }),
    })
    assert.equal(notOk.response.status, 502)
    assert.equal(notOk.response.headers.get('cache-control'), null)

    const timeout = await callHandler({
        channel: 'release',
        onFetch: () => {
            throw new Error('The operation was aborted due to timeout')
        },
    })
    assert.equal(timeout.response.status, 502)
    assert.equal(timeout.response.headers.get('cache-control'), null)
    assert.match(timeout.text, /Failed to fetch GitLink releases/)
})
