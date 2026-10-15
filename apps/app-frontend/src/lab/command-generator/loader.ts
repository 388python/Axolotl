import type { CommandDataFile, CommandDefinition, VersionManifest } from './commands/types.ts'
// 静态 import：JSON 在编译期被打包进 bundle，运行时零 IO 开销
import manifestData from './data/manifest.json'
import v120Data from './data/v1.20.json' // 假设还有其他版本数据文件
import v121Data from './data/v1.21.json'

const dataMap: Record<string, CommandDataFile> = {
    'v1.21.json': v121Data as CommandDataFile,
    'v1.20.json': v120Data as CommandDataFile,
}

let manifestCache: VersionManifest | null = null

/** 同步获取版本清单 */
export function loadManifest(): VersionManifest {
    if (manifestCache) return manifestCache
    manifestCache = manifestData as VersionManifest
    return manifestCache
}

/** 同步获取指定版本的命令数据 */
export function loadCommandsForVersion(versionId: string): CommandDefinition[] {
    const manifest = loadManifest()
    const entry = manifest.versions.find((v) => v.id === versionId)
    if (!entry) {
        throw new Error(`Unknown version: ${versionId}`)
    }

    const data = dataMap[entry.dataFile]
    if (!data) {
        throw new Error(`Data file not registered: ${entry.dataFile}`)
    }

    return data.commands
}
