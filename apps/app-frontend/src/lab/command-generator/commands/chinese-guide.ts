export interface ChineseGuideEntry {
    commandId: string
    summary: string
    examples: { command: string; explanation: string }[]
    tips?: string[]
}

export const CHINESE_GUIDES: ChineseGuideEntry[] = [
    {
        commandId: 'give',
        summary: '给予玩家物品，支持 NBT 自定义。',
        examples: [
            { command: '/give @s minecraft:diamond 64', explanation: '给自己 64 个钻石' },
            {
                command:
                    '/give @p minecraft:diamond_sword{Enchantments:[{id:"minecraft:sharpness",lvl:5}]} 1',
                explanation: '给最近玩家一把锋利 V 的钻石剑',
            },
        ],
        tips: ['数量上限为 99', 'NBT 数据需要用花括号包裹'],
    },
    {
        commandId: 'summon',
        summary: '在指定位置生成实体。',
        examples: [
            {
                command: '/summon minecraft:axolotl ~ ~ ~',
                explanation: '在当前位置生成一只美西螈',
            },
            {
                command: '/summon minecraft:creeper ~ ~ ~ {ExplosionRadius:10}',
                explanation: '生成一只爆炸半径为 10 的爬行者',
            },
        ],
        tips: ['实体 ID 需要带命名空间'],
    },
    {
        commandId: 'gamemode',
        summary: '切换玩家的游戏模式。',
        examples: [
            { command: '/gamemode creative @a', explanation: '将所有玩家切换为创造模式' },
            { command: '/gamemode survival @s', explanation: '将自己切换为生存模式' },
        ],
    },
    {
        commandId: 'effect',
        summary: '给予或移除状态效果。',
        examples: [
            {
                command: '/effect give @s minecraft:speed 30 2',
                explanation: '给自己 30 秒的速度 III',
            },
            {
                command: '/effect clear @a minecraft:poison',
                explanation: '清除所有玩家的中毒效果',
            },
        ],
    },
    {
        commandId: 'time',
        summary: '设置或查询游戏时间。',
        examples: [
            { command: '/time set day', explanation: '设置为白天（1000 刻）' },
            { command: '/time add 1000', explanation: '快进 1000 刻' },
        ],
        tips: ['0=日出，6000=中午，12000=日落，18000=午夜'],
    },
    {
        commandId: 'weather',
        summary: '设置天气。',
        examples: [
            { command: '/weather clear', explanation: '设为晴天' },
            { command: '/weather rain 600', explanation: '设为雨天并持续 10 分钟' },
        ],
    },
    {
        commandId: 'tp',
        summary: '传送实体到指定位置。',
        examples: [
            { command: '/tp @s 100 64 200', explanation: '将自己传送到坐标 (100, 64, 200)' },
            { command: '/tp @p @s', explanation: '将最近玩家传送到自己身边' },
        ],
    },
    {
        commandId: 'execute',
        summary: '以特定上下文执行命令，支持条件判断、实体选择等高级操作。',
        examples: [
            {
                command: '/execute as @a at @s run say hello',
                explanation: '让所有玩家在各自位置说 hello',
            },
            {
                command: '/execute if entity @e[type=zombie] run say 有僵尸',
                explanation: '如果存在僵尸则广播',
            },
        ],
        tips: ['子命令链以 run 结尾', '常用子命令：as/at/positioned/if/unless'],
    },
    {
        commandId: 'fill',
        summary: '用指定方块填充一个区域。',
        examples: [
            {
                command: '/fill 0 64 0 10 64 10 minecraft:stone',
                explanation: '用石头填充一个 11x1x11 的区域',
            },
            {
                command: '/fill 0 64 0 10 64 10 minecraft:glass hollow',
                explanation: '填充玻璃外壳',
            },
        ],
        tips: ['模式：replace/keep/outline/hollow/destroy'],
    },
    {
        commandId: 'setblock',
        summary: '放置单个方块。',
        examples: [
            { command: '/setblock ~ ~1 ~ minecraft:chest', explanation: '在头顶放置一个箱子' },
        ],
    },
]

export function findGuide(commandId: string): ChineseGuideEntry | undefined {
    return CHINESE_GUIDES.find((g) => g.commandId === commandId)
}
