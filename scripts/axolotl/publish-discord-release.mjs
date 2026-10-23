import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { stripTypeScriptTypes } from 'node:module'
import { setTimeout } from 'node:timers/promises'

import { validateReleaseNotices } from './release-notices.mjs'
import { githubRepository, isMain, releaseVersion, verifyRelease } from './release-utils.mjs'

const MiB = 1024 * 1024
const requiredPermissions = 1024n | 2048n | 16384n | 32768n | 65536n
const categoryLabels = {
    added: '新增 / Added',
    changed: '变更 / Changed',
    deprecated: '弃用 / Deprecated',
    removed: '移除 / Removed',
    fixed: 'Bug 修复 / Fixed',
    security: '安全修复 / Security',
}

export function uploadLimit(value = '20') {
    const limit = Number(value)
    // Leave room for multipart metadata within Discord's 25 MiB request limit.
    if (!Number.isFinite(limit) || limit <= 0 || limit > 24) {
        throw new Error('DISCORD_MAX_UPLOAD_MIB must be greater than 0 and at most 24')
    }
    return Math.floor(limit * MiB)
}

export async function loadAnnouncement(source, tag) {
    const version = releaseVersion(tag)
    const javascript = stripTypeScriptTypes(source)
    const catalog = await import(
        `data:text/javascript;base64,${Buffer.from(javascript).toString('base64')}`
    )
    const announcement = catalog.getAnnouncementByVersion(version)
    if (!announcement) throw new Error(`No bundled announcement found for release ${version}`)
    return announcement
}

export function splitDescription(text, limit = 3500) {
    const chunks = []
    let remaining = text.trim()
    while (remaining.length > limit) {
        let end = remaining.lastIndexOf('\n', limit)
        if (end < limit / 2) end = limit
        // Discord counts UTF-16 units; never split a surrogate pair.
        if (/[\uD800-\uDBFF]/.test(remaining[end - 1])) end--
        chunks.push(remaining.slice(0, end))
        remaining = remaining.slice(end).trimStart()
    }
    if (remaining) chunks.push(remaining)
    return chunks
}

export function buildReleaseMessages(release, announcement, { repository, notice = null } = {}) {
    const tag = release.tag_name
    verifyRelease(release, tag, { published: true })
    if (announcement.version !== releaseVersion(tag))
        throw new Error('Announcement version mismatch')
    if (!Number.isSafeInteger(release.id) || release.id <= 0) throw new Error('Invalid release ID')
    const url = `https://github.com/${repository}/releases/tag/${encodeURIComponent(tag)}`
    const title = `Axolotl Launcher ${tag}`
    if (title.length > 220 || url.length > 512)
        throw new Error('Release tag is too long for Discord')
    const pair = (text) => {
        if (!text?.['zh-CN']?.trim() || !text?.['en-US']?.trim()) {
            throw new Error('Announcement text requires both zh-CN and en-US')
        }
        return `${text['zh-CN']}\n${text['en-US']}`
    }
    const lines = [`发布日期 / Published: ${announcement.publishedAt}`, '']
    if (notice) lines.push(`**${notice.type.toUpperCase()}**`, pair(notice), '')
    for (const [category, label] of Object.entries(categoryLabels)) {
        const changes = announcement.changes[category]
        if (!changes?.length) continue
        lines.push(`**${label}**`, '')
        for (const change of changes) lines.push(`- ${pair(change).replaceAll('\n', '\n  ')}`, '')
    }
    if (announcement.notes) lines.push('**说明 / Notes**', pair(announcement.notes))

    const message = (key, description, heading = title) => ({
        key,
        payload: {
            allowed_mentions: { parse: [] },
            embeds: [
                {
                    title: heading,
                    url,
                    color: 0xf28db2,
                    description,
                    footer: { text: `axolotl-release:${repository}:${release.id}:${key}` },
                },
            ],
            components: [
                {
                    type: 1,
                    components: [{ type: 2, style: 5, label: 'GitHub Releases', url }],
                },
            ],
        },
    })
    const chunks = splitDescription(lines.join('\n'))
    const messages = chunks.map((chunk, index) =>
        message(`notes-${index + 1}`, chunk, `${title} (${index + 1}/${chunks.length})`),
    )
    const installers = release.assets.filter((asset) => /\.(exe|dmg|deb)$/i.test(asset.name))
    for (const extension of ['exe', 'dmg', 'deb']) {
        if (!installers.some((asset) => asset.name.toLowerCase().endsWith(`.${extension}`))) {
            throw new Error(`Release is missing a .${extension} installer`)
        }
    }
    for (const asset of installers) {
        if (
            !Number.isSafeInteger(asset.id) ||
            !Number.isSafeInteger(asset.size) ||
            asset.size <= 0
        ) {
            throw new Error(`Invalid installer metadata: ${asset.name}`)
        }
        const description =
            `[${asset.name.replace(/[\[\]\\]/g, '\\$&')}](${asset.browser_download_url})\n` +
            `${(asset.size / MiB).toFixed(1)} MiB`
        if (description.length > 3500) throw new Error('Installer filename is too long for Discord')
        messages.push({ ...message(`asset-${asset.id}`, description), asset })
    }
    return messages
}

export class DiscordError extends Error {
    constructor(status, code) {
        super(`Discord API request failed (HTTP ${status}, code ${code ?? 'unknown'})`)
        this.status = status
        this.code = code
    }
}

export function discordClient(token, { fetchImpl = fetch, sleep = setTimeout } = {}) {
    return async (endpoint, { method = 'GET', payload, file } = {}) => {
        for (let attempt = 0; attempt < 5; attempt++) {
            const headers = {
                Authorization: `Bot ${token}`,
                'User-Agent': 'DiscordBot (https://github.com/Axolotl-Launcher/Axolotl, 1.0)',
            }
            let body
            if (file) {
                body = new FormData()
                body.set('payload_json', JSON.stringify(payload))
                body.set('files[0]', new Blob([file.bytes]), file.name)
            } else if (payload) {
                headers['Content-Type'] = 'application/json'
                body = JSON.stringify(payload)
            }
            let response
            try {
                response = await fetchImpl(`https://discord.com/api/v10${endpoint}`, {
                    method,
                    headers,
                    body,
                    signal: AbortSignal.timeout(120000),
                })
            } catch {
                // POSTs use enforce_nonce, including when a response is lost after delivery.
                if (attempt === 4) throw new Error('Discord API network request failed')
                await sleep(1000 * 2 ** attempt)
                continue
            }
            const data = await response.json().catch(() => ({}))
            if (response.ok) return data
            if ((response.status === 429 || response.status >= 500) && attempt < 4) {
                const seconds = response.status === 429 ? Number(data.retry_after) : 2 ** attempt
                if (!Number.isFinite(seconds) || seconds < 0 || seconds > 120) {
                    throw new Error('Discord retry delay is invalid or exceeds 120 seconds')
                }
                await sleep(Math.ceil(seconds * 1000) + 100)
                continue
            }
            throw new DiscordError(response.status, data.code)
        }
    }
}

export async function downloadInstaller(asset, maxBytes, fetchImpl = fetch) {
    const response = await fetchImpl(asset.browser_download_url, {
        signal: AbortSignal.timeout(120000),
    })
    if (!response.ok) throw new Error(`Installer download failed (HTTP ${response.status})`)
    const chunks = []
    let size = 0
    for await (const chunk of response.body) {
        size += chunk.byteLength
        if (size > maxBytes || size > asset.size) throw new Error('Installer exceeds expected size')
        chunks.push(chunk)
    }
    if (size !== asset.size) throw new Error('Installer download size mismatch')
    const bytes = Buffer.concat(chunks)
    if (asset.digest) {
        const digest = `sha256:${createHash('sha256').update(bytes).digest('hex')}`
        if (asset.digest !== digest) throw new Error('Installer download digest mismatch')
    }
    return bytes
}

export function channelPermissions(channel, roles, member, botId) {
    const roleIds = new Set([channel.guild_id, ...member.roles])
    let permissions = roles
        .filter((role) => roleIds.has(role.id))
        .reduce((value, role) => value | BigInt(role.permissions), 0n)
    if (permissions & 8n) return permissions | requiredPermissions
    const overwrites = channel.permission_overwrites ?? []
    const apply = (allow, deny) => {
        permissions = (permissions & ~deny) | allow
    }
    const everyone = overwrites.find((entry) => entry.id === channel.guild_id)
    if (everyone) apply(BigInt(everyone.allow), BigInt(everyone.deny))
    let allow = 0n
    let deny = 0n
    for (const entry of overwrites) {
        if (entry.type === 0 && entry.id !== channel.guild_id && roleIds.has(entry.id)) {
            allow |= BigInt(entry.allow)
            deny |= BigInt(entry.deny)
        }
    }
    apply(allow, deny)
    const own = overwrites.find((entry) => entry.type === 1 && entry.id === botId)
    if (own) apply(BigInt(own.allow), BigInt(own.deny))
    return permissions
}

export async function publishDiscordRelease({
    release,
    messages,
    channelId,
    api,
    maxUploadBytes = uploadLimit(),
    download = downloadInstaller,
}) {
    if (!/^\d{17,20}$/.test(channelId)) throw new Error('Invalid DISCORD_CHANNEL_ID')
    const bot = await api('/users/@me')
    const channel = await api(`/channels/${channelId}`)
    if (![0, 5].includes(channel.type)) {
        throw new Error('DISCORD_CHANNEL_ID must identify a text or announcement channel')
    }
    const roles = await api(`/guilds/${channel.guild_id}/roles`)
    const member = await api(`/guilds/${channel.guild_id}/members/${bot.id}`)
    // Read Message History is required: without it Discord returns an empty history,
    // which would silently defeat deduplication on retries.
    if (
        (channelPermissions(channel, roles, member, bot.id) & requiredPermissions) !==
        requiredPermissions
    ) {
        throw new Error(
            'Bot needs View Channel, Send Messages, Embed Links, Attach Files and Read Message History',
        )
    }
    const delivered = new Set()
    let before = ''
    do {
        const history = await api(`/channels/${channelId}/messages?limit=100${before}`)
        for (const entry of history) {
            if (entry.author?.id !== bot.id) continue
            for (const embed of entry.embeds ?? []) delivered.add(embed.footer?.text)
        }
        const oldest = history.at(-1)
        if (history.length < 100 || Date.parse(oldest.timestamp) < Date.parse(release.published_at))
            break
        before = `&before=${oldest.id}`
    } while (true)

    let sent = 0
    for (const { payload, asset } of messages) {
        const marker = payload.embeds[0].footer.text
        if (delivered.has(marker)) continue
        const content = structuredClone(payload)
        content.nonce = createHash('sha256').update(marker).digest('hex').slice(0, 24)
        content.enforce_nonce = true
        let file
        if (asset && asset.size <= maxUploadBytes) {
            file = { name: asset.name, bytes: await download(asset, maxUploadBytes) }
            content.attachments = [{ id: 0, filename: asset.name }]
        } else if (asset) {
            content.embeds[0].description +=
                '\n附件超出上传限制，请使用下载链接。\nFile exceeds upload limit; use the download link.'
        }
        try {
            await api(`/channels/${channelId}/messages`, { method: 'POST', payload: content, file })
        } catch (error) {
            if (!file || !(error.status === 413 || error.code === 40005)) throw error
            delete content.attachments
            content.embeds[0].description +=
                '\nDiscord 拒绝了附件大小，请使用下载链接。\nDiscord rejected the attachment size; use the download link.'
            await api(`/channels/${channelId}/messages`, { method: 'POST', payload: content })
        }
        delivered.add(marker)
        sent++
    }
    return { sent, skipped: messages.length - sent }
}

if (isMain(import.meta.url)) {
    const [tag, mode] = process.argv.slice(2)
    if (!tag || (mode && mode !== '--dry-run')) {
        throw new Error('Usage: node publish-discord-release.mjs <tag> [--dry-run]')
    }
    releaseVersion(tag)
    const repository = githubRepository()
    const git = (...args) => execFileSync('git', args, { encoding: 'utf8', maxBuffer: 8 * MiB })
    const ref = `refs/tags/${tag}`
    const announcement = await loadAnnouncement(
        git('show', `${ref}:apps/app-frontend/src/announcements/catalog.ts`),
        tag,
    )
    const noticesPath = '.github/release-notices.json'
    const notice = git('ls-tree', '--name-only', ref, '--', noticesPath).trim()
        ? validateReleaseNotices(JSON.parse(git('show', `${ref}:${noticesPath}`)))[tag]
        : null
    const gh = (endpoint) =>
        JSON.parse(execFileSync('gh', ['api', endpoint], { encoding: 'utf8', maxBuffer: 8 * MiB }))
    const release = gh(`repos/${repository}/releases/tags/${encodeURIComponent(tag)}`)
    release.assets = []
    for (let page = 1; ; page++) {
        const assets = gh(
            `repos/${repository}/releases/${release.id}/assets?per_page=100&page=${page}`,
        )
        release.assets.push(...assets)
        if (assets.length < 100) break
    }
    const messages = buildReleaseMessages(release, announcement, { repository, notice })
    const maxUploadBytes = uploadLimit(process.env.DISCORD_MAX_UPLOAD_MIB)
    if (mode === '--dry-run') {
        console.log(JSON.stringify({ messages, maxUploadBytes }, null, 2))
    } else {
        if (!process.env.DISCORD_BOT_TOKEN) throw new Error('DISCORD_BOT_TOKEN is required')
        const result = await publishDiscordRelease({
            release,
            messages,
            channelId: process.env.DISCORD_CHANNEL_ID,
            api: discordClient(process.env.DISCORD_BOT_TOKEN),
            maxUploadBytes,
        })
        console.log(
            `Discord release ${tag}: ${result.sent} sent, ${result.skipped} already delivered`,
        )
    }
}
