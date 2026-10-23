import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import fs from 'node:fs/promises'
import test from 'node:test'

import {
    buildReleaseMessages,
    channelPermissions,
    discordClient,
    DiscordError,
    downloadInstaller,
    loadAnnouncement,
    publishDiscordRelease,
    splitDescription,
    uploadLimit,
} from './publish-discord-release.mjs'
import { assetDownloadUrl, githubRepository } from './release-utils.mjs'

const repository = githubRepository()
const tag = 'v2.3.4-beta.1'
const channelId = '123456789012345678'
const botId = '234567890123456789'
const guildId = '345678901234567890'
const required = 117760n
const announcement = {
    version: tag.slice(1),
    publishedAt: '2026-01-01',
    changes: { added: [{ 'zh-CN': '新增测试功能', 'en-US': 'Added a test feature' }] },
    notes: { 'zh-CN': '中文说明', 'en-US': 'English notes' },
}
const release = {
    id: 42,
    tag_name: tag,
    draft: false,
    published_at: '2026-01-01T00:00:00Z',
    assets: ['exe', 'dmg', 'deb', 'deb', 'exe.sig', 'rpm'].map((extension, index) => ({
        id: index + 100,
        name: `Axolotl-${index}.${extension}`,
        size: 4,
        browser_download_url: assetDownloadUrl(tag, `Axolotl-${index}.${extension}`),
    })),
}
const plan = (overrides = {}) =>
    buildReleaseMessages(release, announcement, { repository, ...overrides })

function mockDiscord({ history = [], failAt = -1, permission = required, channelType = 0 } = {}) {
    const sent = []
    const reads = []
    const api = async (endpoint, options = {}) => {
        if (endpoint === '/users/@me') return { id: botId }
        if (endpoint === `/channels/${channelId}`) return { type: channelType, guild_id: guildId }
        if (endpoint.endsWith('/roles')) return [{ id: guildId, permissions: String(permission) }]
        if (endpoint.endsWith(`/members/${botId}`)) return { roles: [] }
        if (options.method === 'POST') {
            if (sent.length === failAt) throw new Error('Simulated interruption')
            sent.push(options)
            return { id: String(sent.length) }
        }
        reads.push(endpoint)
        return history[reads.length - 1] ?? []
    }
    return { api, sent, reads }
}

const send = (mock, options = {}) =>
    publishDiscordRelease({
        release,
        messages: plan(),
        channelId,
        api: mock.api,
        download: async () => Buffer.from('test'),
        ...options,
    })
const historical = (payload, id = '777') => ({
    id,
    author: { id: botId },
    embeds: payload.embeds,
    timestamp: '2026-01-02T00:00:00Z',
})

test('renders paired bilingual categories, notices, notes and release buttons', () => {
    const messages = plan({ notice: { type: 'warning', 'zh-CN': '中文警告', 'en-US': 'Warning' } })
    const first = messages[0].payload
    assert.match(first.embeds[0].description, /中文警告\nWarning/)
    assert.match(first.embeds[0].description, /新增 \/ Added/)
    assert.match(first.embeds[0].description, /新增测试功能\n  Added a test feature/)
    assert.match(first.embeds[0].description, /中文说明\nEnglish notes/)
    assert.equal(messages.length, 5)
    assert.deepEqual(
        messages.slice(1).map((entry) => entry.asset.id),
        [100, 101, 102, 103],
    )
    for (const { payload } of messages) {
        assert.deepEqual(payload.allowed_mentions, { parse: [] })
        assert.equal(payload.components[0].components[0].style, 5)
        assert.equal(
            payload.components[0].components[0].url,
            `https://github.com/${repository}/releases/tag/${tag}`,
        )
        assert.ok(payload.embeds[0].description.length <= 4096)
    }
})

test('loads the exact release from TypeScript and rejects a missing version', async () => {
    const source = `const entry: { version: string } = ${JSON.stringify(announcement)}
        export function getAnnouncementByVersion(version: string) {
            return entry.version === version ? entry : undefined
        }`
    assert.deepEqual(await loadAnnouncement(source, tag), announcement)
    await assert.rejects(loadAnnouncement(source, 'v2.3.5'), /No bundled announcement/)
})

test('can read the real catalog without frontend dependencies or changing it', async () => {
    const source = await fs.readFile(
        new URL('../../apps/app-frontend/src/announcements/catalog.ts', import.meta.url),
        'utf8',
    )
    assert.equal((await loadAnnouncement(source, 'v1.4.0')).version, '1.4.0')
})

test('splits large descriptions without dropping text or breaking emoji', () => {
    const text = '中'.repeat(3499) + '🦎'.repeat(4000)
    const chunks = splitDescription(text)
    assert.equal(chunks.join(''), text)
    assert.ok(chunks.every((chunk) => chunk.length <= 3500))
    assert.ok(chunks.every((chunk) => !/[\uD800-\uDBFF]$/.test(chunk)))
    const large = buildReleaseMessages(
        release,
        {
            ...announcement,
            changes: {
                fixed: Array.from({ length: 100 }, () => ({
                    'zh-CN': '修复'.repeat(100),
                    'en-US': 'Fixed '.repeat(100),
                })),
            },
        },
        { repository },
    )
    assert.ok(large.length > 10)
    assert.equal(
        new Set(large.map((entry) => entry.payload.embeds[0].footer.text)).size,
        large.length,
    )
})

test('rejects unpublished, mismatched, incomplete and foreign releases before sending', () => {
    for (const update of [
        { draft: true },
        { published_at: null },
        { tag_name: 'v2.3.5' },
        { assets: release.assets.filter((asset) => !asset.name.endsWith('.dmg')) },
        {
            assets: [
                { ...release.assets[0], browser_download_url: 'https://example.com/file.exe' },
            ],
        },
        { assets: release.assets.map((asset) => ({ ...asset, size: 0 })) },
    ]) {
        assert.throws(() =>
            buildReleaseMessages({ ...release, ...update }, announcement, { repository }),
        )
    }
    assert.throws(() =>
        buildReleaseMessages(release, { ...announcement, version: '2.3.5' }, { repository }),
    )
    assert.throws(
        () =>
            buildReleaseMessages(
                release,
                {
                    ...announcement,
                    notes: { 'zh-CN': '中文' },
                },
                { repository },
            ),
        /both zh-CN and en-US/,
    )
})

test('sends every installer architecture as an attachment and suppresses mentions', async () => {
    const mock = mockDiscord()
    assert.deepEqual(await send(mock), { sent: 5, skipped: 0 })
    assert.equal(mock.sent.filter((entry) => entry.file).length, 4)
    assert.equal(new Set(mock.sent.map((entry) => entry.payload.nonce)).size, 5)
    for (const { payload } of mock.sent) {
        assert.equal(payload.enforce_nonce, true)
        assert.ok(payload.nonce.length <= 25)
        assert.deepEqual(payload.allowed_mentions, { parse: [] })
    }
})

test('oversized installers retain direct links without downloading files', async () => {
    const mock = mockDiscord()
    await send(mock, { maxUploadBytes: 1, download: () => assert.fail('Must not download') })
    for (const entry of mock.sent.slice(1)) {
        assert.equal(entry.file, undefined)
        assert.match(entry.payload.embeds[0].description, /File exceeds upload limit/)
        assert.match(entry.payload.embeds[0].description, /github.com/)
    }
})

test('Discord size rejection falls back to links, while authentication errors fail', async () => {
    for (const error of [new DiscordError(413), new DiscordError(400, 40005)]) {
        const mock = mockDiscord()
        await send(mock, {
            api: async (endpoint, options) => {
                if (options?.file) throw error
                return mock.api(endpoint, options)
            },
        })
        assert.equal(mock.sent.length, 5)
        assert.match(mock.sent[1].payload.embeds[0].description, /Discord rejected/)
    }
    const mock = mockDiscord()
    await assert.rejects(
        send(mock, {
            api: async (endpoint, options) => {
                if (options?.method === 'POST') throw new DiscordError(403, 50013)
                return mock.api(endpoint, options)
            },
        }),
        /HTTP 403/,
    )
})

test('reruns paginate history and skip only this bot’s previously delivered parts', async () => {
    const messages = plan()
    const filler = Array.from({ length: 100 }, (_, index) => ({
        id: String(index),
        author: { id: 'other-bot' },
        embeds: messages[0].payload.embeds,
        timestamp: '2026-01-02T00:00:00Z',
    }))
    const mock = mockDiscord({ history: [filler, [historical(messages[0].payload)]] })
    assert.deepEqual(await send(mock), { sent: 4, skipped: 1 })
    assert.match(mock.reads[1], /before=99/)
    const complete = mockDiscord({ history: [messages.map(({ payload }) => historical(payload))] })
    assert.deepEqual(await send(complete, { download: () => assert.fail('Must not download') }), {
        sent: 0,
        skipped: 5,
    })
})

test('resumes after partial delivery without resending completed messages', async () => {
    const partial = mockDiscord({ failAt: 2 })
    await assert.rejects(send(partial), /Simulated interruption/)
    const resumed = mockDiscord({
        history: [partial.sent.map(({ payload }) => historical(payload))],
    })
    assert.deepEqual(await send(resumed), { sent: 3, skipped: 2 })
})

test('requires history permission and a supported channel before posting', async () => {
    const missing = mockDiscord({ permission: required & ~65536n })
    await assert.rejects(send(missing), /Read Message History/)
    assert.equal(missing.sent.length, 0)
    await assert.rejects(send(mockDiscord({ channelType: 15 })), /text or announcement channel/)
    await assert.rejects(
        send(mockDiscord(), { channelId: 'invalid' }),
        /Invalid DISCORD_CHANNEL_ID/,
    )
})

test('applies everyone, aggregate role and member permission overwrites in order', () => {
    const channel = {
        guild_id: guildId,
        permission_overwrites: [
            { id: guildId, type: 0, allow: '0', deny: '65536' },
            { id: 'role', type: 0, allow: '65536', deny: '0' },
            { id: 'role2', type: 0, allow: '0', deny: '65536' },
        ],
    }
    const roles = [{ id: guildId, permissions: String(required) }]
    const member = { roles: ['role', 'role2'] }
    assert.equal(channelPermissions(channel, roles, member, botId), required)
    channel.permission_overwrites.push({ id: botId, type: 1, allow: '0', deny: '65536' })
    assert.equal(channelPermissions(channel, roles, member, botId), required & ~65536n)
    assert.equal(
        channelPermissions(channel, [{ id: guildId, permissions: '8' }], member, botId) & required,
        required,
    )
})

test('uploads valid bytes and rejects truncated, excessive or tampered downloads', async () => {
    const bytes = Buffer.from('test')
    const asset = {
        ...release.assets[0],
        digest: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
    }
    const download = (body) => async () => new Response(body)
    assert.deepEqual(await downloadInstaller(asset, 10, download(bytes)), bytes)
    await assert.rejects(downloadInstaller(asset, 2, download(bytes)), /exceeds expected size/)
    await assert.rejects(downloadInstaller(asset, 10, download('tes')), /size mismatch/)
    await assert.rejects(
        downloadInstaller(asset, 10, download('test extra')),
        /exceeds expected size/,
    )
    await assert.rejects(downloadInstaller(asset, 10, download('evil')), /digest mismatch/)
})

test('uses multipart payload_json and files[0], without manually setting its boundary', async () => {
    const api = discordClient('test-token', {
        fetchImpl: async (url, options) => {
            assert.equal(url, 'https://discord.com/api/v10/channels/123/messages')
            assert.equal(options.headers.Authorization, 'Bot test-token')
            assert.equal(options.headers['Content-Type'], undefined)
            assert.deepEqual(JSON.parse(options.body.get('payload_json')), { content: 'hello' })
            assert.equal(await options.body.get('files[0]').text(), 'test')
            assert.equal(options.body.get('files[0]').name, 'Axolotl.exe')
            return Response.json({ id: 'sent' })
        },
    })
    await api('/channels/123/messages', {
        method: 'POST',
        payload: { content: 'hello' },
        file: { name: 'Axolotl.exe', bytes: Buffer.from('test') },
    })
})

test('honors Discord rate limits and retries transient failures with the same nonce', async () => {
    const delays = []
    const payloads = []
    let calls = 0
    const api = discordClient('test-token', {
        sleep: async (delay) => delays.push(delay),
        fetchImpl: async (_url, options) => {
            calls++
            payloads.push(options.body)
            if (calls === 1) return Response.json({ retry_after: 1.25 }, { status: 429 })
            if (calls === 2) throw new Error('Network error containing sensitive data')
            if (calls === 3) return Response.json({}, { status: 503 })
            return Response.json({ id: 'sent' })
        },
    })
    const payload = { nonce: 'same', enforce_nonce: true }
    await api('/channels/123/messages', { method: 'POST', payload })
    assert.deepEqual(delays, [1350, 2000, 4100])
    assert.ok(payloads.every((body) => body === JSON.stringify(payload)))
})

test('bounds retries, never retries permanent failures and never logs raw API errors', async () => {
    let calls = 0
    const api = discordClient('test-token', {
        sleep: async () => {},
        fetchImpl: async () => {
            calls++
            return Response.json({ code: 50013, message: 'sensitive data' }, { status: 403 })
        },
    })
    await assert.rejects(api('/users/@me'), (error) => {
        assert.equal(error.message, 'Discord API request failed (HTTP 403, code 50013)')
        return true
    })
    assert.equal(calls, 1)
    calls = 0
    const failing = discordClient('test-token', {
        sleep: async () => {},
        fetchImpl: async () => {
            calls++
            return Response.json({}, { status: 503 })
        },
    })
    await assert.rejects(failing('/users/@me'), /HTTP 503/)
    assert.equal(calls, 5)
})

test('upload configuration respects the total request ceiling', () => {
    assert.equal(uploadLimit(), 20 * 1024 * 1024)
    assert.equal(uploadLimit('24'), 24 * 1024 * 1024)
    for (const value of ['0', '-1', '25', 'NaN', 'Infinity', '']) {
        assert.throws(() => uploadLimit(value), /DISCORD_MAX_UPLOAD_MIB/)
    }
})
