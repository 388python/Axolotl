import { loadCommandsForVersion, loadManifest } from '../loader.ts'
import type { CommandCategory, CommandDefinition } from './types.ts'

let currentCommands: CommandDefinition[] = []
let currentVersion = '1.21'
let commandIndex = new Map<string, CommandDefinition>()

export function initializeRegistry(versionId?: string): void {
    const manifest = loadManifest()
    currentVersion = versionId ?? manifest.defaultVersion
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

export function getCommandsByCategory(
    category: CommandCategory,
): CommandDefinition[] {
    return currentCommands.filter((c) => c.category === category)
}

export const CATEGORY_KEYS: Record<CommandCategory, string> = {
    player: 'command-generator.cat.player',
    world: 'command-generator.cat.world',
    entity: 'command-generator.cat.entity',
    block: 'command-generator.cat.block',
    item: 'command-generator.cat.item',
    communication: 'command-generator.cat.communication',
    server: 'command-generator.cat.server',
    data: 'command-generator.cat.data',
    misc: 'command-generator.cat.misc',
}

export const PERMISSION_KEYS: Record<number, string> = {
    0: 'command-generator.perm.0',
    1: 'command-generator.perm.1',
    2: 'command-generator.perm.2',
    3: 'command-generator.perm.3',
    4: 'command-generator.perm.4',
}

export function searchCommands(query: string): CommandDefinition[] {
    if (!query.trim()) return currentCommands
    const q = query.toLowerCase().trim()
    return currentCommands.filter((cmd) => {
        if (cmd.name.toLowerCase().includes(q)) return true
        if (cmd.aliases?.some((a) => a.toLowerCase().includes(q))) return true
        return false
    })
}
