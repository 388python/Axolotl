<script setup lang="ts">
import { Combobox, type ComboboxOption, useVIntl } from '@modrinth/ui'
import { computed, ref } from 'vue'

import type { ArgumentPreset, CommandArgument } from '@/lab/command-generator/commands/types.ts'
import { commandGeneratorMessages as messages } from '@/lab/command-generator/i18n/messages.ts'

import CommandGeneratorNbtEditor from './CommandGeneratorNbtEditor.vue'

const props = defineProps<{
    arg: CommandArgument
    modelValue: string | number | boolean | undefined
}>()

const emit = defineEmits<{
    'update:modelValue': [value: string | number | boolean]
}>()

const { formatMessage } = useVIntl()

function t(key: string, fallback?: string): string {
    return formatMessage({ id: key, defaultMessage: fallback ?? key })
}

const ENTITY_SELECTOR_DEFAULT_PRESETS: ArgumentPreset[] = [
    { labelKey: 'command-generator.preset.entity.self', value: '@s' },
    { labelKey: 'command-generator.preset.entity.nearest-player', value: '@p' },
    { labelKey: 'command-generator.preset.entity.all-players', value: '@a' },
    { labelKey: 'command-generator.preset.entity.random-player', value: '@r' },
    { labelKey: 'command-generator.preset.entity.all-entities', value: '@e' },
    { labelKey: 'command-generator.preset.entity.nearest-entity', value: '@n' },
    { labelKey: 'command-generator.preset.entity.zombies', value: '@e[type=zombie]' },
    { labelKey: 'command-generator.preset.entity.skeletons', value: '@e[type=skeleton]' },
    { labelKey: 'command-generator.preset.entity.creepers', value: '@e[type=creeper]' },
    { labelKey: 'command-generator.preset.entity.endermen', value: '@e[type=enderman]' },
    { labelKey: 'command-generator.preset.entity.villagers', value: '@e[type=villager]' },
    { labelKey: 'command-generator.preset.entity.items', value: '@e[type=item]' },
    { labelKey: 'command-generator.preset.entity.creative-players', value: '@a[gamemode=creative]' },
    { labelKey: 'command-generator.preset.entity.survival-players', value: '@a[gamemode=survival]' },
    { labelKey: 'command-generator.preset.entity.nearby-10', value: '@p[distance=..10]' },
    { labelKey: 'command-generator.preset.entity.nearby-entities-5', value: '@e[distance=..5]' },
]

const isNumber = computed(() => props.arg.type === 'number' || props.arg.type === 'integer')
const isNbt = computed(() => props.arg.supportsNbtEditor === true)
const isEnumDropdown = computed(
    () => !!props.arg.enumValues && props.arg.enumValues.length > 0,
)

const enumOptions = computed<ComboboxOption[]>(() =>
    (props.arg.enumValues ?? []).map((value) => ({ value, label: value })),
)

const mergedPresets = computed<ArgumentPreset[]>(() => {
    const custom = props.arg.presets ?? []
    if (props.arg.type !== 'entity-selector') return custom

    const seen = new Set<string>()
    const result: ArgumentPreset[] = []

    for (const p of custom) {
        const key = String(p.value)
        if (!seen.has(key)) {
            seen.add(key)
            result.push(p)
        }
    }

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
            <span class="argument-name">{{ t(arg.displayNameKey) }}</span>
            <span v-if="arg.required" class="argument-required">
                {{ formatMessage(messages.argRequired) }}
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
                    {{ formatMessage(messages.argSelectPlaceholder) }}
                </span>
            </template>
        </Combobox>

        <label v-else-if="arg.type === 'boolean'" class="argument-toggle">
            <input
                type="checkbox"
                :checked="modelValue === true || modelValue === 'true'"
                @change="onCheckboxChange(($event.target as HTMLInputElement).checked)"
            />
            <span>{{ t(arg.descriptionKey) }}</span>
        </label>

        <input
            v-else
            :value="modelValue"
            :type="isNumber ? 'number' : 'text'"
            :placeholder="arg.placeholder ?? t(arg.descriptionKey)"
            class="argument-input"
            @input="onTextInput(($event.target as HTMLInputElement).value)"
        />

        <div v-if="showPresets" class="argument-presets">
            <button
                v-for="preset in visiblePresets"
                :key="preset.labelKey"
                type="button"
                class="preset-chip"
                :title="preset.descriptionKey ? t(preset.descriptionKey) : undefined"
                @click="applyPreset(preset.value)"
            >
                {{ t(preset.labelKey) }}
            </button>
            <button
                v-if="hiddenPresetCount > 0 && !showAllPresets"
                type="button"
                class="preset-chip preset-chip--more"
                @click="showAllPresets = true"
            >
                +{{ hiddenPresetCount }}
            </button>
        </div>

        <p v-if="!isNbt && arg.type !== 'boolean'" class="argument-hint">
            {{ t(arg.descriptionKey) }}
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
