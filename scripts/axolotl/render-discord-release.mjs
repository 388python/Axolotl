import { createRequire } from 'node:module'

const require = createRequire(new URL('../../apps/app-frontend/package.json', import.meta.url))

export const categoryLabels = {
    added: '新增 / Added',
    changed: '变更 / Changed',
    deprecated: '弃用 / Deprecated',
    removed: '移除 / Removed',
    fixed: 'Bug 修复 / Fixed',
    security: '安全修复 / Security',
}

const escape = (value) =>
    String(value).replace(
        /[&<>"']/g,
        (character) =>
            ({
                '&': '&amp;',
                '<': '&lt;',
                '>': '&gt;',
                '"': '&quot;',
                "'": '&#39;',
            })[character],
    )

function localizedRow(text) {
    for (const locale of ['zh-CN', 'en-US']) {
        if (!text?.[locale]?.trim())
            throw new Error('Announcement text requires both zh-CN and en-US')
    }
    const inline = (value) =>
        escape(value)
            .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
            .replace(/`([^`]+)`/g, '<code>$1</code>')
            .replace(/\[([^\]]+)\]\(https?:\/\/[^)]+\)/g, '$1')
    return `<div class="row"><div lang="zh-CN">${inline(text['zh-CN'])}</div><div lang="en">${inline(text['en-US'])}</div></div>`
}

export function buildReleaseNotesHtml(announcement, { notice = null } = {}) {
    let count = 0
    const sections = Object.entries(categoryLabels).flatMap(([key, label]) => {
        const changes = announcement.changes[key]
        if (!changes?.length) return []
        count += changes.length
        return [
            `<section><h2>${label}<span>${changes.length}</span></h2>${changes.map(localizedRow).join('')}</section>`,
        ]
    })
    if (notice)
        sections.unshift(
            `<section class="notice"><h2>${escape(notice.type.toUpperCase())}</h2>${localizedRow(notice)}</section>`,
        )
    if (announcement.notes)
        sections.push(`<section><h2>说明 / Notes</h2>${localizedRow(announcement.notes)}</section>`)
    return `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<style>
* { box-sizing: border-box; }
body { margin: 0; width: 1440px; background: #111319; color: #edf0f5; font: 22px/1.5 'Noto Sans CJK SC', 'Microsoft YaHei', sans-serif; }
main { padding: 40px 44px 30px; border-top: 8px solid #ef9bbe; }
header { display: flex; align-items: center; justify-content: space-between; gap: 24px; padding-bottom: 25px; }
.brand { color: #ef9bbe; font-size: 17px; font-weight: 700; letter-spacing: 2px; }
h1 { margin: 7px 0; font-size: 42px; line-height: 1.2; letter-spacing: -1px; overflow-wrap: anywhere; }
.meta { color: #b5bbc8; font-size: 18px; }
.badge { color: #f9cee0; background: #382a35; padding: 8px 17px; border-radius: 8px; font-size: 18px; white-space: nowrap; }
.languages, .row { display: grid; grid-template-columns: 1fr 1fr; gap: 32px; }
.languages { padding: 10px 18px; color: #aeb5c4; font-size: 17px; border-bottom: 1px solid #353843; }
section { margin-top: 20px; }
h2 { margin: 0; padding: 0 16px 10px; font-size: 23px; color: #f4b6d0; display: flex; align-items: center; gap: 12px; }
h2 span { color: #b5bbc8; font-size: 16px; font-weight: 400; }
.row { padding: 10px 18px; border-radius: 7px; }
.row:nth-child(even) { background: #1c1f27; }
.row > div { white-space: pre-wrap; overflow-wrap: anywhere; }
.row > div[lang=en] { color: #c8ced9; font-size: 20px; }
.notice { background: #302932; padding: 12px 0; border-radius: 8px; }
code { color: #f4b6d0; font: .9em monospace; }
footer { margin-top: 28px; border-top: 1px solid #353843; padding-top: 16px; display: flex; justify-content: space-between; color: #929aaa; font-size: 16px; }
</style></head><body><main>
<header><div><div class="brand">AXOLOTL LAUNCHER</div><h1>${escape(announcement.version)}</h1><div class="meta">${escape(announcement.publishedAt)} · ${count} 项更新 / changes</div></div><div class="badge">${announcement.version.includes('-') ? '预发布 / Beta' : '正式版 / Release'}</div></header>
<div class="languages"><div>中文</div><div>ENGLISH</div></div>
${sections.join('')}
<footer><span>Axolotl Launcher · 更新日志 / Release notes</span><span>安装包与下载链接见消息 / Downloads in message</span></footer>
</main></body></html>`
}

export async function renderReleaseNotesImage(html, { channel } = {}) {
    const { chromium } = require('playwright')
    const browser = await chromium.launch({ headless: true, ...(channel ? { channel } : {}) })
    try {
        const page = await browser.newPage({
            viewport: { width: 1440, height: 900 },
            deviceScaleFactor: 1,
        })
        await page.route('**/*', (route) => route.abort())
        await page.setContent(html, { waitUntil: 'load' })
        await page.evaluate(() => document.fonts.ready)
        const height = await page
            .locator('main')
            .evaluate((element) => Math.ceil(element.getBoundingClientRect().height))
        if (height > 30000) throw new Error('Release notes exceed the 30000px image height limit')
        return await page.locator('main').screenshot({ type: 'png', animations: 'disabled' })
    } finally {
        await browser.close()
    }
}
