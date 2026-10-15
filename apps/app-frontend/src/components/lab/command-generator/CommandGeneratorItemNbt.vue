<script setup lang="ts">
import { PlusIcon, SparklesIcon, XIcon } from '@modrinth/assets'
import { Combobox, type ComboboxOption, defineMessages, Toggle, useVIntl } from '@modrinth/ui'
import { computed, ref, watch } from 'vue'

interface EnchantEntry {
    key: number
    id: string
    level: number
}

interface EnchantDefinition {
    id: string
    label: string
    maxLevel: number
}

const props = defineProps<{
    modelValue?: string
}>()

const emit = defineEmits<{
    'update:modelValue': [value: string]
}>()

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: {
        id: 'app.lab.command-generator.item-nbt.title',
        defaultMessage: 'Item customisation',
    },
    nameLabel: {
        id: 'app.lab.command-generator.item-nbt.name-label',
        defaultMessage: 'Custom name',
    },
    nameHint: {
        id: 'app.lab.command-generator.item-nbt.name-hint',
        defaultMessage: 'Shown as the item title.',
    },
    namePlaceholder: {
        id: 'app.lab.command-generator.item-nbt.name-placeholder',
        defaultMessage: 'Legendary sword',
    },
    loreLabel: {
        id: 'app.lab.command-generator.item-nbt.lore-label',
        defaultMessage: 'Lore lines',
    },
    loreHint: {
        id: 'app.lab.command-generator.item-nbt.lore-hint',
        defaultMessage: 'Each line appears under the item name.',
    },
    lorePlaceholder: {
        id: 'app.lab.command-generator.item-nbt.lore-placeholder',
        defaultMessage: 'A line of lore…',
    },
    addLore: {
        id: 'app.lab.command-generator.item-nbt.add-lore',
        defaultMessage: 'Add line',
    },
    enchLabel: {
        id: 'app.lab.command-generator.item-nbt.ench-label',
        defaultMessage: 'Enchantments',
    },
    enchHint: {
        id: 'app.lab.command-generator.item-nbt.ench-hint',
        defaultMessage: 'Pick an enchantment and set its level.',
    },
    enchAdd: {
        id: 'app.lab.command-generator.item-nbt.ench-add',
        defaultMessage: 'Add enchantment',
    },
    enchLevel: {
        id: 'app.lab.command-generator.item-nbt.ench-level',
        defaultMessage: 'Level',
    },
    enchEmpty: {
        id: 'app.lab.command-generator.item-nbt.ench-empty',
        defaultMessage: 'No enchantments yet.',
    },
    unbreakableLabel: {
        id: 'app.lab.command-generator.item-nbt.unbreakable-label',
        defaultMessage: 'Unbreakable',
    },
    unbreakableHint: {
        id: 'app.lab.command-generator.item-nbt.unbreakable-hint',
        defaultMessage: 'Prevents the item from taking durability damage.',
    },
    previewLabel: {
        id: 'app.lab.command-generator.item-nbt.preview-label',
        defaultMessage: 'NBT preview',
    },
    previewEmpty: {
        id: 'app.lab.command-generator.item-nbt.preview-empty',
        defaultMessage: 'Empty — no NBT will be applied.',
    },
})

const ENCHANTMENTS: EnchantDefinition[] = [
    { id: 'minecraft:sharpness', label: '锋利', maxLevel: 5 },
    { id: 'minecraft:smite', label: '亡灵杀手', maxLevel: 5 },
    { id: 'minecraft:bane_of_arthropods', label: '节肢杀手', maxLevel: 5 },
    { id: 'minecraft:knockback', label: '击退', maxLevel: 2 },
    { id: 'minecraft:fire_aspect', label: '火焰附加', maxLevel: 2 },
    { id: 'minecraft:looting', label: '抢夺', maxLevel: 3 },
    { id: 'minecraft:sweeping_edge', label: '横扫之刃', maxLevel: 3 },
    { id: 'minecraft:efficiency', label: '效率', maxLevel: 5 },
    { id: 'minecraft:silk_touch', label: '精准采集', maxLevel: 1 },
    { id: 'minecraft:unbreaking', label: '耐久', maxLevel: 3 },
    { id: 'minecraft:fortune', label: '时运', maxLevel: 3 },
    { id: 'minecraft:mending', label: '经验修补', maxLevel: 1 },
    { id: 'minecraft:power', label: '力量', maxLevel: 5 },
    { id: 'minecraft:punch', label: '冲击', maxLevel: 2 },
    { id: 'minecraft:flame', label: '火矢', maxLevel: 1 },
    { id: 'minecraft:infinity', label: '无限', maxLevel: 1 },
    { id: 'minecraft:multishot', label: '多重射击', maxLevel: 1 },
    { id: 'minecraft:quick_charge', label: '快速装填', maxLevel: 3 },
    { id: 'minecraft:piercing', label: '穿透', maxLevel: 4 },
    { id: 'minecraft:protection', label: '保护', maxLevel: 4 },
    { id: 'minecraft:fire_protection', label: '火焰保护', maxLevel: 4 },
    { id: 'minecraft:feather_falling', label: '摔落保护', maxLevel: 4 },
    { id: 'minecraft:blast_protection', label: '爆炸保护', maxLevel: 4 },
    { id: 'minecraft:projectile_protection', label: '弹射物保护', maxLevel: 4 },
    { id: 'minecraft:respiration', label: '水下呼吸', maxLevel: 3 },
    { id: 'minecraft:aqua_affinity', label: '水下速掘', maxLevel: 1 },
    { id: 'minecraft:thorns', label: '荆棘', maxLevel: 3 },
    { id: 'minecraft:depth_strider', label: '深海探索者', maxLevel: 3 },
    { id: 'minecraft:frost_walker', label: '冰霜行者', maxLevel: 2 },
    { id: 'minecraft:binding_curse', label: '绑定诅咒', maxLevel: 1 },
    { id: 'minecraft:vanishing_curse', label: '消失诅咒', maxLevel: 1 },
    { id: 'minecraft:soul_speed', label: '灵魂疾行', maxLevel: 3 },
    { id: 'minecraft:swift_sneak', label: '迅捷潜行', maxLevel: 3 },
    { id: 'minecraft:loyalty', label: '忠诚', maxLevel: 3 },
    { id: 'minecraft:impaling', label: '穿刺', maxLevel: 5 },
    { id: 'minecraft:riptide', label: '激流', maxLevel: 3 },
    { id: 'minecraft:channeling', label: '引雷', maxLevel: 1 },
    { id: 'minecraft:luck_of_the_sea', label: '海之眷顾', maxLevel: 3 },
    { id: 'minecraft:lure', label: '饵钓', maxLevel: 3 },
]

const enchantOptions: ComboboxOption[] = ENCHANTMENTS.map((e) => ({
    value: e.id,
    label: e.label,
}))

const enchantById = computed(() => new Map(ENCHANTMENTS.map((e) => [e.id, e])))

const name = ref('')
const lore = ref<string[]>([])
const enchantments = ref<EnchantEntry[]>([])
const unbreakable = ref(false)
let nextKey = 1

const snbt = computed(() => serialize())

watch(snbt, (val) => emit('update:modelValue', val), { immediate: true })

watch(
    () => props.modelValue,
    (val) => {
        if (
            !val &&
            (name.value || lore.value.length || enchantments.value.length || unbreakable.value)
        ) {
            name.value = ''
            lore.value = []
            enchantments.value = []
            unbreakable.value = false
        }
    },
)

function serialize(): string {
    const parts: string[] = []

    const displayParts: string[] = []
    if (name.value.trim()) {
        displayParts.push(`Name:'{"text":"${escapeSnbt(name.value)}"}'`)
    }
    const cleanLore = lore.value.map((l) => l.trim()).filter(Boolean)
    if (cleanLore.length) {
        const loreItems = cleanLore.map((l) => `'{"text":"${escapeSnbt(l)}"}'`).join(',')
        displayParts.push(`Lore:[${loreItems}]`)
    }
    if (displayParts.length) {
        parts.push(`display:{${displayParts.join(',')}}`)
    }

    if (enchantments.value.length) {
        const items = enchantments.value.map((e) => `{id:"${e.id}",lvl:${e.level}}`).join(',')
        parts.push(`Enchantments:[${items}]`)
    }

    if (unbreakable.value) {
        parts.push('Unbreakable:1b')
    }

    return parts.length ? `{${parts.join(',')}}` : ''
}

function escapeSnbt(value: string): string {
    return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
}

function addLore() {
    lore.value.push('')
}

function removeLore(index: number) {
    lore.value.splice(index, 1)
}

function addEnchant() {
    const used = new Set(enchantments.value.map((e) => e.id))
    const next = ENCHANTMENTS.find((e) => !used.has(e.id))
    if (!next) return
    enchantments.value.push({ key: nextKey++, id: next.id, level: 1 })
}

function removeEnchant(index: number) {
    enchantments.value.splice(index, 1)
}

function setEnchantId(index: number, id: unknown) {
    const entry = enchantments.value[index]
    if (!entry) return
    entry.id = String(id)
    const def = enchantById.value.get(entry.id)
    if (def && entry.level > def.maxLevel) entry.level = def.maxLevel
}

function setEnchantLevel(index: number, level: number) {
    const entry = enchantments.value[index]
    if (!entry) return
    const def = enchantById.value.get(entry.id)
    const max = def?.maxLevel ?? 10
    const min = 1
    entry.level = Math.min(max, Math.max(min, Math.floor(level || min)))
}

function enchantLabel(id: string): string {
    return enchantById.value.get(id)?.label ?? id
}
</script>

<template>
    <div class="item-nbt">
        <header class="item-nbt-head">
            <span class="item-nbt-title">
                <SparklesIcon />
                {{ formatMessage(messages.title) }}
            </span>
        </header>

        <div class="field">
            <label class="field-label">
                <span>{{ formatMessage(messages.nameLabel) }}</span>
                <span class="field-hint">{{ formatMessage(messages.nameHint) }}</span>
            </label>
            <input
                v-model="name"
                class="text-input"
                :placeholder="formatMessage(messages.namePlaceholder)"
            />
        </div>

        <div class="field">
            <label class="field-label">
                <span>{{ formatMessage(messages.loreLabel) }}</span>
                <span class="field-hint">{{ formatMessage(messages.loreHint) }}</span>
            </label>
            <div class="lore-list">
                <div v-for="(_, i) in lore" :key="i" class="lore-row">
                    <input
                        v-model="lore[i]"
                        class="text-input"
                        :placeholder="formatMessage(messages.lorePlaceholder)"
                    />
                    <button type="button" class="icon-btn icon-btn--danger" @click="removeLore(i)">
                        <XIcon />
                    </button>
                </div>
                <button type="button" class="ghost-add" @click="addLore">
                    <PlusIcon />
                    {{ formatMessage(messages.addLore) }}
                </button>
            </div>
        </div>

        <div class="field">
            <label class="field-label">
                <span>{{ formatMessage(messages.enchLabel) }}</span>
                <span class="field-hint">{{ formatMessage(messages.enchHint) }}</span>
            </label>
            <div class="enchant-list">
                <div v-if="!enchantments.length" class="enchant-empty">
                    {{ formatMessage(messages.enchEmpty) }}
                </div>
                <div v-for="(ench, i) in enchantments" :key="ench.key" class="enchant-row">
                    <div class="enchant-select">
                        <Combobox
                            :model-value="ench.id"
                            :options="enchantOptions"
                            searchable
                            @update:model-value="setEnchantId(i, $event)"
                        >
                            <template #selected="{ label }">
                                <span class="truncate">
                                    {{ label ?? enchantLabel(ench.id) }}
                                </span>
                            </template>
                        </Combobox>
                    </div>
                    <div class="enchant-level">
                        <span class="enchant-level-label">
                            {{ formatMessage(messages.enchLevel) }}
                        </span>
                        <input
                            class="text-input text-input--num"
                            type="number"
                            min="1"
                            :max="enchantById.get(ench.id)?.maxLevel ?? 10"
                            :value="ench.level"
                            @input="
                                setEnchantLevel(
                                    i,
                                    Number(($event.target as HTMLInputElement).value),
                                )
                            "
                        />
                    </div>
                    <button
                        type="button"
                        class="icon-btn icon-btn--danger"
                        @click="removeEnchant(i)"
                    >
                        <XIcon />
                    </button>
                </div>
                <button type="button" class="ghost-add" @click="addEnchant">
                    <PlusIcon />
                    {{ formatMessage(messages.enchAdd) }}
                </button>
            </div>
        </div>

        <div class="field field--row">
            <div class="toggle-text">
                <label class="field-label">
                    <span>{{ formatMessage(messages.unbreakableLabel) }}</span>
                    <span class="field-hint">
                        {{ formatMessage(messages.unbreakableHint) }}
                    </span>
                </label>
            </div>
            <Toggle v-model="unbreakable" />
        </div>

        <div class="preview">
            <span class="preview-label">{{ formatMessage(messages.previewLabel) }}</span>
            <code v-if="snbt" class="preview-code">{{ snbt }}</code>
            <span v-else class="preview-empty">{{ formatMessage(messages.previewEmpty) }}</span>
        </div>
    </div>
</template>

<style scoped>
.item-nbt {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    padding: 0.9rem 0.95rem;
}

.item-nbt-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.item-nbt-title {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
    font-weight: 800;
    color: var(--color-text-primary);
}

.item-nbt-title :deep(svg) {
    width: 0.9rem;
    height: 0.9rem;
    color: var(--color-brand);
}

.field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.field--row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
}

.field-label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.74rem;
    font-weight: 700;
    color: var(--color-text-primary);
}

.field-hint {
    font-size: 0.63rem;
    font-weight: 400;
    color: var(--color-text-tertiary);
}

.text-input {
    min-width: 0;
    flex: 1;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-1);
    padding: 0.45rem 0.65rem;
    color: var(--color-text-primary);
    font-size: 0.76rem;
    outline: none;
    transition: border-color 0.15s ease;
}

.text-input:focus {
    border-color: var(--color-brand);
}

.text-input--num {
    flex: 0 0 auto;
    width: 4.5rem;
    text-align: center;
}

.lore-list,
.enchant-list {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.lore-row,
.enchant-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
}

.enchant-select {
    min-width: 0;
    flex: 1;
}

.enchant-level {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex: 0 0 auto;
}

.enchant-level-label {
    font-size: 0.65rem;
    color: var(--color-text-tertiary);
}

.enchant-empty {
    border: 1px dashed var(--color-divider);
    border-radius: var(--radius-sm);
    padding: 0.5rem 0.65rem;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.icon-btn {
    display: inline-flex;
    width: 1.6rem;
    height: 1.6rem;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-tertiary);
    cursor: pointer;
    transition:
        background 0.15s ease,
        color 0.15s ease;
}

.icon-btn:hover {
    background: var(--surface-4);
}

.icon-btn--danger:hover {
    color: var(--color-red);
}

.icon-btn :deep(svg) {
    width: 0.78rem;
    height: 0.78rem;
}

.ghost-add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.3rem;
    border: 1px dashed var(--color-divider);
    border-radius: var(--radius-sm);
    background: transparent;
    padding: 0.4rem 0.6rem;
    color: var(--color-text-tertiary);
    font-size: 0.7rem;
    font-weight: 700;
    cursor: pointer;
    transition:
        border-color 0.15s ease,
        color 0.15s ease;
}

.ghost-add:hover {
    border-color: var(--color-brand);
    color: var(--color-brand);
}

.ghost-add :deep(svg) {
    width: 0.75rem;
    height: 0.75rem;
}

.toggle-text {
    min-width: 0;
    flex: 1;
}

.preview {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    border-top: 1px solid var(--color-divider);
    padding-top: 0.7rem;
}

.preview-label {
    font-size: 0.62rem;
    font-weight: 800;
    text-transform: uppercase;
    color: var(--color-text-tertiary);
}

.preview-code {
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--surface-1);
    padding: 0.55rem 0.65rem;
    color: var(--color-text-secondary);
    font-family: var(--mono-font, monospace);
    font-size: 0.68rem;
    white-space: pre-wrap;
    word-break: break-all;
}

.preview-empty {
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}
</style>
