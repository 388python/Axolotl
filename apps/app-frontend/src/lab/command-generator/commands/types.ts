export type ArgumentType =
    | 'string'
    | 'number'
    | 'integer'
    | 'boolean'
    | 'enum'
    | 'coordinates'
    | 'entity-selector'
    | 'item'
    | 'block'
    | 'nbt'
    | 'message'
    | 'time'
    | 'gamemode'
    | 'difficulty'
    | 'biome'
    | 'particle'
    | 'sound'
    | 'effect'
    | 'enchantment'
    | 'attribute'
    | 'slot'
    | 'objective'
    | 'team'

export type NbtKind = 'item' | 'entity' | 'generic'

export interface ArgumentPreset {
    label: string
    value: string | number | boolean
    description?: string
}

export interface CommandArgument {
    name: string
    displayName: string
    type: ArgumentType
    required: boolean
    description: string
    enumValues?: string[]
    defaultValue?: string | number | boolean
    selector?: boolean
    placeholder?: string
    supportsNbtEditor?: boolean
    nbtKind?: NbtKind
    presets?: ArgumentPreset[]
}

// ============ Execute 链式构建器专用 ============

export interface ExecuteChainSubcommand {
    id: string
    label: string
    description: string
    /** 命令模板，使用 {argName} 引用参数值 */
    template: string
    arguments: CommandArgument[]
}

export interface ExecuteChainCategory {
    id: string
    label: string
    subcommands: ExecuteChainSubcommand[]
}

export interface CommandDefinition {
    id: string
    name: string
    description: string
    category: CommandCategory
    permissionLevel: 0 | 1 | 2 | 3 | 4
    arguments: CommandArgument[]
    aliases?: string[]
    sinceVersion?: string
    removedVersion?: string
    /** 是否为链式构建命令（如 execute） */
    isChainCommand?: boolean
    /** 链式子命令分类（仅 isChainCommand 时有效） */
    chainCategories?: ExecuteChainCategory[]
}

export type CommandCategory =
    'player' | 'world' | 'entity' | 'block' | 'item' | 'communication' | 'server' | 'data' | 'misc'

export interface CommandDataFile {
    version: string
    generatedAt: string
    commands: CommandDefinition[]
}

export interface VersionManifest {
    versions: VersionEntry[]
    default: string
}

export interface VersionEntry {
    id: string
    label: string
    dataFile: string
}
