// Netlify Function：把 GitLink 镜像的 release 列表归一化为与 releases-latest 相同的形状。
// 浏览器直连 GitLink 会被 CORS 拦截（无 Access-Control-Allow-Origin），由本站服务器代为拉取。
//
// GitLink 与 GitHub 的差异（详见交接文档）：
// - 业务错误也返回 HTTP 200，必须校验 body.status === 0；
// - draft 是字符串（'草稿' 表示草稿），没有 prerelease 字段，预发布只能靠 tag 名判断；
// - attachments[].url 是相对路径，需要拼 https://www.gitlink.org.cn；
// - attachments[].filesize 是本地化字符串，这里不依赖它。
const GITLINK_ORIGIN = 'https://www.gitlink.org.cn'
const GITLINK_RELEASES_URL = `${GITLINK_ORIGIN}/api/xinvxueyuan/Axolotl/releases.json`
const CHANNELS = ['release', 'beta']

function semverCore(tag) {
	const core = tag.replace(/^v/, '').split('-')[0]
	return core.split('.').map((part) => Number.parseInt(part, 10) || 0)
}

// 按 tag 逐段比较，不依赖 GitLink 的 created_at（本地时间、无时区）。
// 同版本号时预发布排在稳定版之前，保证 release 频道不会把 beta 当成稳定版。
function compareTags(a, b) {
	const left = semverCore(a)
	const right = semverCore(b)
	const length = Math.max(left.length, right.length)
	for (let index = 0; index < length; index += 1) {
		const difference = (left[index] ?? 0) - (right[index] ?? 0)
		if (difference !== 0) return difference
	}
	const leftPrerelease = a.includes('-')
	const rightPrerelease = b.includes('-')
	if (leftPrerelease === rightPrerelease) return a.localeCompare(b)
	return leftPrerelease ? 1 : -1
}

function selectTag(releases, channel) {
	const matches = releases.filter((release) =>
		channel === 'release' ? !release.tag_name.includes('-') : release.tag_name.includes('-'),
	)
	if (!matches.length) return null
	return matches.sort((a, b) => compareTags(a.tag_name, b.tag_name)).at(-1)
}

function toAsset(attachment) {
	const name = attachment?.title
	if (typeof name !== 'string' || !name) return null
	const path = typeof attachment.url === 'string' ? attachment.url : ''
	return {
		name,
		// 前端用这个地址直接向 GitLink 下载，不再经过本函数。
		url: /^https?:\/\//i.test(path) ? path : `${GITLINK_ORIGIN}${path}`,
	}
}

function readChannel(request) {
	try {
		const channel = new URL(request.url).searchParams.get('channel')
		if (channel === null) return 'release'
		return CHANNELS.includes(channel) ? channel : null
	} catch {
		const channel = request.queryStringParameters?.channel
		if (channel === undefined) return 'release'
		return CHANNELS.includes(channel) ? channel : null
	}
}

export default async (request) => {
	const channel = readChannel(request)
	if (!channel) {
		return new Response('Unsupported release channel', { status: 400 })
	}

	try {
		const response = await fetch(GITLINK_RELEASES_URL, {
			headers: {
				Accept: 'application/json',
				'User-Agent': 'Axolotl-Website',
			},
			signal: AbortSignal.timeout(8000),
		})
		if (!response.ok) {
			return new Response(`GitLink source returned ${response.status}`, { status: 502 })
		}

		const payload = await response.json()
		if (payload?.status !== 0 || !Array.isArray(payload.releases)) {
			return new Response('Invalid GitLink releases payload', { status: 502 })
		}

		const published = payload.releases.filter(
			(release) => typeof release?.tag_name === 'string' && release.draft !== '草稿',
		)
		const top = selectTag(published, channel)
		if (!top) {
			return new Response(`No ${channel} release on GitLink`, { status: 502 })
		}

		const normalized = {
			tag_name: top.tag_name,
			assets: (Array.isArray(top.attachments) ? top.attachments : [])
				.map(toAsset)
				.filter(Boolean),
		}

		return new Response(JSON.stringify(normalized), {
			headers: {
				'Content-Type': 'application/json',
				// 浏览器不缓存；GitLink 自身不提供 304，Netlify CDN 缓存 5 分钟以减少回源。
				// 错误响应不带该头，因此不会被缓存。
				'Cache-Control': 'public, max-age=0, s-maxage=300',
			},
		})
	} catch (error) {
		return new Response(`Failed to fetch GitLink releases: ${error.message}`, { status: 502 })
	}
}
