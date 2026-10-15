export interface CommandGuideEntry {
    commandId: string
    summaryKey: string
    examples: { command: string; explanationKey: string }[]
    tipKeys?: string[]
}

export const COMMAND_GUIDES: CommandGuideEntry[] = [
    {
        commandId: 'give',
        summaryKey: 'command-generator.guide.give.summary',
        examples: [
            {
                command: '/give @s minecraft:diamond 64',
                explanationKey: 'command-generator.guide.give.ex1',
            },
            {
                command: '/give @p minecraft:diamond_sword{Enchantments:[{id:"minecraft:sharpness",lvl:5}]} 1',
                explanationKey: 'command-generator.guide.give.ex2',
            },
        ],
        tipKeys: [
            'command-generator.guide.give.tip1',
            'command-generator.guide.give.tip2',
        ],
    },
    {
        commandId: 'summon',
        summaryKey: 'command-generator.guide.summon.summary',
        examples: [
            {
                command: '/summon minecraft:axolotl ~ ~ ~',
                explanationKey: 'command-generator.guide.summon.ex1',
            },
            {
                command: '/summon minecraft:creeper ~ ~ ~ {ExplosionRadius:10}',
                explanationKey: 'command-generator.guide.summon.ex2',
            },
        ],
        tipKeys: ['command-generator.guide.summon.tip1'],
    },
    {
        commandId: 'gamemode',
        summaryKey: 'command-generator.guide.gamemode.summary',
        examples: [
            {
                command: '/gamemode creative @a',
                explanationKey: 'command-generator.guide.gamemode.ex1',
            },
            {
                command: '/gamemode survival @s',
                explanationKey: 'command-generator.guide.gamemode.ex2',
            },
        ],
    },
    {
        commandId: 'effect',
        summaryKey: 'command-generator.guide.effect.summary',
        examples: [
            {
                command: '/effect give @s minecraft:speed 30 2',
                explanationKey: 'command-generator.guide.effect.ex1',
            },
            {
                command: '/effect clear @a minecraft:poison',
                explanationKey: 'command-generator.guide.effect.ex2',
            },
        ],
    },
    {
        commandId: 'time',
        summaryKey: 'command-generator.guide.time.summary',
        examples: [
            {
                command: '/time set day',
                explanationKey: 'command-generator.guide.time.ex1',
            },
            {
                command: '/time add 1000',
                explanationKey: 'command-generator.guide.time.ex2',
            },
        ],
        tipKeys: ['command-generator.guide.time.tip1'],
    },
    {
        commandId: 'weather',
        summaryKey: 'command-generator.guide.weather.summary',
        examples: [
            {
                command: '/weather clear',
                explanationKey: 'command-generator.guide.weather.ex1',
            },
            {
                command: '/weather rain 600',
                explanationKey: 'command-generator.guide.weather.ex2',
            },
        ],
    },
    {
        commandId: 'tp',
        summaryKey: 'command-generator.guide.tp.summary',
        examples: [
            {
                command: '/tp @s 100 64 200',
                explanationKey: 'command-generator.guide.tp.ex1',
            },
            {
                command: '/tp @p @s',
                explanationKey: 'command-generator.guide.tp.ex2',
            },
        ],
    },
    {
        commandId: 'teleport',
        summaryKey: 'command-generator.guide.tp.summary',
        examples: [
            {
                command: '/teleport @s 100 64 200',
                explanationKey: 'command-generator.guide.tp.ex1',
            },
        ],
    },
    {
        commandId: 'execute',
        summaryKey: 'command-generator.guide.execute.summary',
        examples: [
            {
                command: '/execute as @a at @s run say hello',
                explanationKey: 'command-generator.guide.execute.ex1',
            },
            {
                command: '/execute if entity @e[type=zombie] run say zombies',
                explanationKey: 'command-generator.guide.execute.ex2',
            },
        ],
        tipKeys: [
            'command-generator.guide.execute.tip1',
            'command-generator.guide.execute.tip2',
        ],
    },
    {
        commandId: 'fill',
        summaryKey: 'command-generator.guide.fill.summary',
        examples: [
            {
                command: '/fill 0 64 0 10 64 10 minecraft:stone',
                explanationKey: 'command-generator.guide.fill.ex1',
            },
            {
                command: '/fill 0 64 0 10 64 10 minecraft:glass hollow',
                explanationKey: 'command-generator.guide.fill.ex2',
            },
        ],
        tipKeys: ['command-generator.guide.fill.tip1'],
    },
    {
        commandId: 'setblock',
        summaryKey: 'command-generator.guide.setblock.summary',
        examples: [
            {
                command: '/setblock ~ ~1 ~ minecraft:chest',
                explanationKey: 'command-generator.guide.setblock.ex1',
            },
        ],
    },
]

export function findGuide(commandId: string): CommandGuideEntry | undefined {
    return COMMAND_GUIDES.find((g) => g.commandId === commandId)
}
