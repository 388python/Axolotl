import { loadCommandsForVersion, loadManifest } from '../loader.ts'
import type { CommandCategory, CommandDefinition } from './types.ts'

let currentCommands: CommandDefinition[] = []
let currentVersion = '1.21'
let commandIndex = new Map<string, CommandDefinition>()

export function initializeRegistry(versionId?: string): void {
    const manifest = loadManifest()
    currentVersion = versionId ?? manifest.default
    currentCommands = loadCommandsForVersion(currentVersion)
    commandIndex = new Map()
    for (const cmd of currentCommands) {
        commandIndex.set(cmd.id, cmd)
    }
}

export function switchVersion(versionId: string): void {
    initializeRegistry(versionId)
}

export function getCurrentVersion(): string {
    return currentVersion
}

export function getAllCommands(): readonly CommandDefinition[] {
    return currentCommands
}

export function findCommand(id: string): CommandDefinition | undefined {
    return commandIndex.get(id)
}

export function getCommandsByCategory(category: CommandCategory): CommandDefinition[] {
    return currentCommands.filter((c) => c.category === category)
}

export const CATEGORY_LABELS: Record<CommandCategory, string> = {
    player: '玩家',
    world: '世界',
    entity: '实体',
    block: '方块',
    item: '物品',
    communication: '通信',
    server: '服务器',
    data: '数据',
    misc: '其他',
}

export function searchCommands(query: string): CommandDefinition[] {
    if (!query.trim()) return currentCommands
    const q = query.toLowerCase().trim()
    return currentCommands.filter((cmd) => {
        if (cmd.name.toLowerCase().includes(q)) return true
        if (cmd.description.toLowerCase().includes(q)) return true
        if (cmd.aliases?.some((a) => a.toLowerCase().includes(q))) return true
        return false
    })
}

// ============================================================
// 兼容旧接口：旧代码用 VANILLA_COMMANDS / COMMANDS_BY_CATEGORY
// 用 Proxy 转发到当前命令数组，无论谁访问都返回最新数据
// ============================================================

export const VANILLA_COMMANDS: CommandDefinition[] = new Proxy([] as CommandDefinition[], {
    get(_target, prop, receiver) {
        return Reflect.get(currentCommands, prop, receiver)
    },
    has(_target, prop) {
        return Reflect.has(currentCommands, prop)
    },
    ownKeys() {
        return Reflect.ownKeys(currentCommands)
    },
    getOwnPropertyDescriptor(_target, prop) {
        return Reflect.getOwnPropertyDescriptor(currentCommands, prop)
    },
}) as CommandDefinition[]

export const COMMANDS_BY_CATEGORY: Record<CommandCategory, CommandDefinition[]> = new Proxy(
    {} as Record<CommandCategory, CommandDefinition[]>,
    {
        get(_target, prop) {
            return currentCommands.filter((c) => c.category === prop)
        },
        has(_target, prop) {
            return typeof prop === 'string'
        },
        ownKeys() {
            return [
                'player',
                'world',
                'entity',
                'block',
                'item',
                'communication',
                'server',
                'data',
                'misc',
            ]
        },
        getOwnPropertyDescriptor() {
            return { enumerable: true, configurable: true }
        },
    },
)

// 模块加载时初始化一次，保证进入页面就有数据
initializeRegistry()
