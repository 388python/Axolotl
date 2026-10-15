<script setup lang="ts">
import { Combobox, type ComboboxOption, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, ref } from 'vue'

import type { ArgumentPreset, CommandArgument } from '@/lab/command-generator/commands/types.ts'

import CommandGeneratorNbtEditor from './CommandGeneratorNbtEditor.vue'

const props = defineProps<{
    arg: CommandArgument
    modelValue: string | number | boolean | undefined
}>()

const emit = defineEmits<{
    'update:modelValue': [value: string | number | boolean]
}>()

const { formatMessage } = useVIntl()

const messages = defineMessages({
    selectPlaceholder: {
        id: 'app.lab.command-generator.arg.select-placeholder',
        defaultMessage: 'Select…',
    },
    required: {
        id: 'app.lab.command-generator.arg.required',
        defaultMessage: 'Required',
    },
})

/**
 * 所有 entity-selector 类型的参数都会自动获得这组预设。
 * 用户点击芯片即可填入对应的目标选择器。
 */
const ENTITY_SELECTOR_DEFAULT_PRESETS: ArgumentPreset[] = [
    // 基础选择器
    { label: '@s 自己', value: '@s', description: '自己' },
    { label: '@p 最近玩家', value: '@p', description: '离命令执行者最近的玩家' },
    { label: '@a 所有玩家', value: '@a', description: '所有在线玩家' },
    { label: '@r 随机玩家', value: '@r', description: '随机一个玩家' },
    { label: '@e 所有实体', value: '@e', description: '所有实体' },
    { label: '@n 最近实体', value: '@n', description: '离命令执行者最近的实体' },

    // 按实体类型
    { label: '僵尸', value: '@e[type=zombie]', description: '@e[type=zombie]' },
    { label: '骷髅', value: '@e[type=skeleton]', description: '@e[type=skeleton]' },
    { label: '苦力怕', value: '@e[type=creeper]', description: '@e[type=creeper]' },
    { label: '末影人', value: '@e[type=enderman]', description: '@e[type=enderman]' },
    { label: '凋灵', value: '@e[type=wither]', description: '@e[type=wither]' },
    { label: '末影龙', value: '@e[type=ender_dragon]', description: '@e[type=ender_dragon]' },
    { label: '村民', value: '@e[type=villager]', description: '@e[type=villager]' },
    { label: '铁傀儡', value: '@e[type=iron_golem]', description: '@e[type=iron_golem]' },
    { label: '掉落物', value: '@e[type=item]', description: '掉在地上的物品' },
    { label: '经验球', value: '@e[type=experience_orb]', description: '经验球' },
    { label: '箭', value: '@e[type=arrow]', description: '飞行中的箭' },
    { label: '船', value: '@e[type=boat]', description: '所有船' },

    // 按游戏模式
    { label: '创造玩家', value: '@a[gamemode=creative]', description: '创造模式玩家' },
    { label: '生存玩家', value: '@a[gamemode=survival]', description: '生存模式玩家' },
    { label: '冒险玩家', value: '@a[gamemode=adventure]', description: '冒险模式玩家' },
    { label: '旁观玩家', value: '@a[gamemode=spectator]', description: '旁观模式玩家' },

    // 按距离
    { label: '附近玩家 10 格', value: '@p[distance=..10]', description: '10 格内的最近玩家' },
    { label: '附近实体 5 格', value: '@e[distance=..5]', description: '5 格内的所有实体' },
    { label: '附近实体 20 格', value: '@e[distance=..20]', description: '20 格内的所有实体' },

    // 按等级 / 经验
    { label: '等级 30+', value: '@a[level=30..]', description: '等级 30 以上的玩家' },
    { label: '等级 0-10', value: '@a[level=0..10]', description: '等级 0 到 10 的玩家' },

    // 按队伍
    { label: '红队', value: '@a[team=red]', description: '红队的玩家' },
    { label: '蓝队', value: '@a[team=blue]', description: '蓝队的玩家' },

    // 组合条件
    {
        label: '10 格内的僵尸',
        value: '@e[type=zombie,distance=..10]',
        description: '10 格内的僵尸',
    },
    {
        label: '20 格内的敌人',
        value: '@e[type=#minecraft:raiders,distance=..20]',
        description: '20 格内的袭击者',
    },
    { label: '所有幼年僵尸', value: '@e[type=zombie,is_baby=true]', description: '幼年僵尸' },
    { label: '所有成年僵尸', value: '@e[type=zombie,is_baby=false]', description: '成年僵尸' },
    { label: '第一个玩家', value: '@a[limit=1]', description: '限制为 1 个玩家' },
]

const isNumber = computed(() => props.arg.type === 'number' || props.arg.type === 'integer')
const isNbt = computed(() => props.arg.supportsNbtEditor === true)
const isEnumDropdown = computed(() => !!props.arg.enumValues && props.arg.enumValues.length > 0)

const enumOptions = computed<ComboboxOption[]>(() =>
    (props.arg.enumValues ?? []).map((value) => ({ value, label: value })),
)

/** 合并自定义预设和默认预设（仅 entity-selector 类型附加默认） */
const mergedPresets = computed<ArgumentPreset[]>(() => {
    const custom = props.arg.presets ?? []

    if (props.arg.type !== 'entity-selector') return custom

    const seen = new Set<string>()
    const result: ArgumentPreset[] = []

    // 自定义预设优先
    for (const p of custom) {
        const key = String(p.value)
        if (!seen.has(key)) {
            seen.add(key)
            result.push(p)
        }
    }

    // 追加默认预设
    for (const p of ENTITY_SELECTOR_DEFAULT_PRESETS) {
        const key = String(p.value)
        if (!seen.has(key)) {
            seen.add(key)
            result.push(p)
        }
    }

    return result
})

const showPresets = computed(
    () => !isEnumDropdown.value && !isNbt.value && mergedPresets.value.length > 0,
)

const VISIBLE_PRESET_COUNT = 4
const showAllPresets = ref(false)
const visiblePresets = computed(() => {
    const list = mergedPresets.value
    if (showAllPresets.value || list.length <= VISIBLE_PRESET_COUNT) return list
    return list.slice(0, VISIBLE_PRESET_COUNT)
})
const hiddenPresetCount = computed(() => {
    const list = mergedPresets.value
    return list.length > VISIBLE_PRESET_COUNT ? list.length - VISIBLE_PRESET_COUNT : 0
})

function onEnumChange(value: unknown) {
    emit('update:modelValue', String(value))
}

function onTextInput(value: string) {
    emit('update:modelValue', isNumber.value ? Number(value) : value)
}

function onCheckboxChange(checked: boolean) {
    emit('update:modelValue', checked)
}

function applyPreset(value: string | number | boolean) {
    emit('update:modelValue', value)
}
</script>

<template>
    <div class="argument-card" :class="{ 'argument-card--nbt': isNbt }">
        <header class="argument-head">
            <span class="argument-name">{{ arg.displayName }}</span>
            <span v-if="arg.required" class="argument-required">
                {{ formatMessage(messages.required) }}
            </span>
            <span class="argument-type">{{ arg.type }}</span>
        </header>

        <CommandGeneratorNbtEditor
            v-if="isNbt"
            :model-value="String(modelValue ?? '')"
            :kind="arg.nbtKind"
            @update:model-value="emit('update:modelValue', $event)"
        />

        <Combobox
            v-else-if="isEnumDropdown"
            :model-value="String(modelValue ?? '')"
            :options="enumOptions"
            @update:model-value="onEnumChange"
        >
            <template #selected="{ label }">
                <span v-if="label" class="truncate">{{ label }}</span>
                <span v-else class="argument-placeholder">
                    {{ formatMessage(messages.selectPlaceholder) }}
                </span>
            </template>
        </Combobox>

        <label v-else-if="arg.type === 'boolean'" class="argument-toggle">
            <input
                type="checkbox"
                :checked="modelValue === true || modelValue === 'true'"
                @change="onCheckboxChange(($event.target as HTMLInputElement).checked)"
            />
            <span>{{ arg.description }}</span>
        </label>

        <input
            v-else
            :value="modelValue"
            :type="isNumber ? 'number' : 'text'"
            :placeholder="arg.placeholder ?? arg.description"
            class="argument-input"
            @input="onTextInput(($event.target as HTMLInputElement).value)"
        />

        <div v-if="showPresets" class="argument-presets">
            <button
                v-for="preset in visiblePresets"
                :key="preset.label"
                type="button"
                class="preset-chip"
                :title="preset.description"
                @click="applyPreset(preset.value)"
            >
                {{ preset.label }}
            </button>
            <button
                v-if="hiddenPresetCount > 0 && !showAllPresets"
                type="button"
                class="preset-chip preset-chip--more"
                @click="showAllPresets = true"
            >
                +{{ hiddenPresetCount }}
            </button>
            <button
                v-if="showAllPresets && mergedPresets.length > VISIBLE_PRESET_COUNT"
                type="button"
                class="preset-chip preset-chip--more"
                @click="showAllPresets = false"
            >
                收起
            </button>
        </div>

        <p v-if="!isNbt && arg.type !== 'boolean'" class="argument-hint">
            {{ arg.description }}
        </p>
    </div>
</template>

<style scoped>
.argument-card {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.55rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 0.9rem 1rem;
}

.argument-card--nbt {
    grid-column: 1 / -1;
}

.argument-head {
    display: flex;
    align-items: center;
    gap: 0.45rem;
}

.argument-name {
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--color-text-primary);
}

.argument-required {
    border-radius: 999px;
    background: var(--surface-4);
    border: 1px solid var(--color-red);
    padding: 0.08rem 0.45rem;
    font-size: 0.6rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--color-red);
}

.argument-type {
    margin-left: auto;
    font-family: var(--mono-font, monospace);
    font-size: 0.65rem;
    color: var(--color-text-tertiary);
}

.argument-input {
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-1);
    padding: 0.5rem 0.7rem;
    color: var(--color-text-primary);
    font-family: var(--mono-font, monospace);
    font-size: 0.8rem;
    outline: none;
    transition: border-color 0.15s ease;
}

.argument-input:focus {
    border-color: var(--color-brand);
}

.argument-toggle {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    font-size: 0.75rem;
    color: var(--color-text-secondary);
}

.argument-toggle input {
    accent-color: var(--color-brand);
}

.argument-placeholder {
    color: var(--color-text-tertiary);
}

.argument-hint {
    margin: 0;
    font-size: 0.68rem;
    line-height: 1.5;
    color: var(--color-text-tertiary);
}

.argument-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.1rem;
}

.preset-chip {
    border: 1px solid var(--color-divider);
    border-radius: 999px;
    background: var(--surface-1);
    padding: 0.28rem 0.65rem;
    color: var(--color-text-secondary);
    font-size: 0.7rem;
    cursor: pointer;
    transition:
        border-color 0.15s ease,
        background 0.15s ease,
        color 0.15s ease;
}

.preset-chip:hover {
    border-color: var(--color-brand);
    background: var(--surface-4);
    color: var(--color-brand);
}

.preset-chip--more {
    font-weight: 700;
    color: var(--color-text-tertiary);
}
</style>
