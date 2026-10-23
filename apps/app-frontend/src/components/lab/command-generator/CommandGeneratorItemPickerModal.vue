<script setup lang="ts">
import { defineMessages, NewModal, useVIntl } from '@modrinth/ui'
import { ref, useTemplateRef } from 'vue'

import advancementsData from '@/lab/command-generator/data/advancements.json'
import effectsData from '@/lab/command-generator/data/effects.json'
import { getSlotDisplay } from '@/lab/recipe-generator/display'
import { resolveRecipeItemName } from '@/lab/recipe-generator/item-names.ts'
import {
    buildSlotContext,
    loadVersionResources,
    TEXTURE_ATLAS,
    type TextureAtlas,
} from '@/lab/recipe-generator/resources'
import type { JavaVersionId, SlotValue } from '@/lab/recipe-generator/types'

import CommandGeneratorItemPicker, { type CommandItemEntry } from './CommandGeneratorItemPicker.vue'

interface EffectData {
    name: string
    iconItem?: string
    displayName?: string
}

interface AdvancementData {
    id: string
    iconItem?: string
    display?: { icon?: { item?: string } }
}

interface ExtraBlockMeta {
    id: string
    nameKey: string
}

interface SourceItem {
    id: string
    name?: string
    __extraMeta?: ExtraBlockMeta
}

const props = defineProps<{
    version: string
    kind?: 'item' | 'block' | 'effect' | 'advancement'
}>()

const emit = defineEmits<{
    select: [value: string]
}>()

const { formatMessage, locale } = useVIntl()
const modal = useTemplateRef<InstanceType<typeof NewModal>>('modal')
const entries = ref<CommandItemEntry[]>([])
const loading = ref(false)
const cachedLocale = ref('')
const atlas: TextureAtlas = TEXTURE_ATLAS

const messages = defineMessages({
    titleItem: {
        id: 'app.lab.command-generator.item-picker.title',
        defaultMessage: 'Pick an item',
    },
    titleBlock: {
        id: 'app.lab.command-generator.block-picker.title',
        defaultMessage: 'Pick a block',
    },
    titleEffect: {
        id: 'app.lab.command-generator.effect-picker.title',
        defaultMessage: 'Pick a status effect',
    },
    titleAdvancement: {
        id: 'app.lab.command-generator.advancement-picker.title',
        defaultMessage: 'Pick an advancement',
    },
})

const EXTRA_BLOCKS_META: ExtraBlockMeta[] = [
    { id: 'minecraft:air', nameKey: 'app.lab.command-generator.block.air' },
    { id: 'minecraft:cave_air', nameKey: 'app.lab.command-generator.block.cave_air' },
    { id: 'minecraft:void_air', nameKey: 'app.lab.command-generator.block.void_air' },
    { id: 'minecraft:water', nameKey: 'app.lab.command-generator.block.water' },
    { id: 'minecraft:lava', nameKey: 'app.lab.command-generator.block.lava' },
    { id: 'minecraft:fire', nameKey: 'app.lab.command-generator.block.fire' },
    { id: 'minecraft:nether_portal', nameKey: 'app.lab.command-generator.block.nether_portal' },
    { id: 'minecraft:end_portal', nameKey: 'app.lab.command-generator.block.end_portal' },
    { id: 'minecraft:end_gateway', nameKey: 'app.lab.command-generator.block.end_gateway' },
    { id: 'minecraft:bubble_column', nameKey: 'app.lab.command-generator.block.bubble_column' },
    { id: 'minecraft:moving_piston', nameKey: 'app.lab.command-generator.block.moving_piston' },
    { id: 'minecraft:piston_head', nameKey: 'app.lab.command-generator.block.piston_head' },
]

function isNonBlockItem(rawId: string): boolean {
    const name = rawId.includes(':') ? rawId.split(':')[1] : rawId
    if (!name) return true

    const ACTUAL_BLOCKS = new Set([
        'kelp',
        'bamboo',
        'sugar_cane',
        'turtle_egg',
        'frogspawn',
        'sculk_vein',
        'cocoa',
        'nether_wart',
        'chorus_flower',
        'sea_pickle',
        'tall_grass',
        'short_grass',
        'fern',
        'large_fern',
        'dead_bush',
        'sweet_berry_bush',
        'crimson_roots',
        'warped_roots',
        'nether_sprouts',
        'chorus_plant',
        'weeping_vines',
        'twisting_vines',
        'vine',
        'glow_lichen',
        'hanging_roots',
        'big_dripleaf',
        'small_dripleaf',
        'spore_blossom',
        'azalea',
        'flowering_azalea',
        'mangrove_propagule',
        'pink_petals',
        'torchflower_seeds',
    ])
    if (ACTUAL_BLOCKS.has(name)) return false

    if (name.endsWith('_spawn_egg') || name === 'spawn_egg') return true
    if (name.endsWith('_bucket') || name === 'bucket') return true
    if (name.endsWith('_horse_armor') || name === 'wolf_armor') return true
    if (name.endsWith('_boat') || name.endsWith('_chest_boat')) return true
    if (name.endsWith('_raft') || name.endsWith('_chest_raft')) return true
    if (name.endsWith('_minecart')) return true
    if (name.startsWith('music_disc_')) return true
    if (name.endsWith('_pottery_sherd')) return true
    if (name.endsWith('_smithing_template')) return true
    if (name.endsWith('_banner_pattern')) return true
    if (
        name.includes('potion') ||
        name.includes('tipped_arrow') ||
        name === 'arrow' ||
        name === 'spectral_arrow'
    )
        return true
    if (/(_sword|_pickaxe|_shovel|_hoe|_axe|_helmet|_chestplate|_leggings|_boots)$/.test(name))
        return true
    if (/(_ingot|_nugget|_dust|_dye|_seeds|_shard|_scute|_sherd|_fragment)$/.test(name)) return true

    const NON_BLOCKS = new Set([
        'apple',
        'golden_apple',
        'enchanted_golden_apple',
        'bread',
        'beef',
        'cooked_beef',
        'porkchop',
        'cooked_porkchop',
        'chicken',
        'cooked_chicken',
        'mutton',
        'cooked_mutton',
        'rabbit',
        'cooked_rabbit',
        'cod',
        'cooked_cod',
        'salmon',
        'cooked_salmon',
        'tropical_fish',
        'pufferfish',
        'potato',
        'baked_potato',
        'poisonous_potato',
        'carrot',
        'golden_carrot',
        'beetroot',
        'beetroot_soup',
        'wheat',
        'sugar',
        'melon_slice',
        'cookie',
        'cake',
        'pumpkin_pie',
        'chorus_fruit',
        'popped_chorus_fruit',
        'honey_bottle',
        'mushroom_stew',
        'rabbit_stew',
        'suspicious_stew',
        'rotten_flesh',
        'spider_eye',
        'fermented_spider_eye',
        'dried_kelp',
        'glistering_melon_slice',
        'sweet_berries',
        'glow_berries',
        'stick',
        'coal',
        'charcoal',
        'diamond',
        'emerald',
        'lapis_lazuli',
        'quartz',
        'amethyst_shard',
        'echo_shard',
        'netherite_scrap',
        'brick',
        'nether_brick',
        'clay_ball',
        'flint',
        'feather',
        'leather',
        'rabbit_hide',
        'string',
        'slime_ball',
        'magma_cream',
        'blaze_rod',
        'breeze_rod',
        'blaze_powder',
        'gunpowder',
        'fire_charge',
        'ink_sac',
        'glow_ink_sac',
        'bone',
        'bone_meal',
        'nautilus_shell',
        'shulker_shell',
        'prismarine_shard',
        'prismarine_crystals',
        'phantom_membrane',
        'dragon_breath',
        'glowstone_dust',
        'redstone',
        'paper',
        'book',
        'writable_book',
        'written_book',
        'name_tag',
        'lead',
        'item_frame',
        'glow_item_frame',
        'painting',
        'armor_stand',
        'experience_bottle',
        'firework_rocket',
        'firework_star',
        'nether_star',
        'heart_of_the_sea',
        'disc_fragment_5',
        'trial_key',
        'ominous_trial_key',
        'armadillo_scute',
        'turtle_scute',
        'wind_charge',
        'raw_iron',
        'raw_gold',
        'raw_copper',
        'bundle',
        'spyglass',
        'brush',
        'goat_horn',
        'map',
        'filled_map',
        'compass',
        'recovery_compass',
        'clock',
        'honeycomb',
        'knowledge_book',
        'ender_pearl',
        'ender_eye',
        'saddle',
        'snowball',
        'egg',
        'ominous_bottle',
        'heavy_core',
        'glass_bottle',
        'rabbit_foot',
        'ghast_tear',
        'enchanted_book',
        'bowl',
        'totem_of_undying',
        'debug_stick',
        'command_block_minecart',
        'shield',
        'bow',
        'crossbow',
        'trident',
        'mace',
        'shears',
        'fishing_rod',
        'flint_and_steel',
        'carrot_on_a_stick',
        'warped_fungus_on_a_stick',
        'elytra',
    ])

    return NON_BLOCKS.has(name)
}

function isVersionAtLeast(current: string, target: string): boolean {
    const cur = current.split('.').map(Number)
    const tgt = target.split('.').map(Number)
    const len = Math.max(cur.length, tgt.length)
    for (let i = 0; i < len; i++) {
        const c = cur[i] || 0
        const t = tgt[i] || 0
        if (c !== t) return c > t
    }
    return true
}

async function show() {
    modal.value?.show()
    if (entries.value.length > 0 && cachedLocale.value === locale.value) return

    loading.value = true
    try {
        const version = props.version as JavaVersionId
        const resources = await loadVersionResources(version)
        const ctx = buildSlotContext([], [], resources)
        const currentLocale = locale.value
        const anyRes = resources as Record<string, unknown> & {
            lang?: Record<string, string>
            effectsByName?: Record<string, EffectData>
            effects?: Record<string, EffectData>
            advancements?: Record<string, AdvancementData>
        }

        const getI18nOrLang = (key: string, langKey: string, fallback: string) => {
            const translated = formatMessage({ id: key, defaultMessage: '' })
            if (translated) return translated
            return anyRes.lang?.[langKey] || fallback
        }

        // 1. 效果
        if (props.kind === 'effect') {
            let effectList: EffectData[] = []
            const effectSource = anyRes.effectsByName || anyRes.effects
            if (effectSource) {
                effectList = Object.values(effectSource || {}) as EffectData[]
            } else {
                effectList = effectsData
                    .filter((e) => isVersionAtLeast(props.version, e.since))
                    .map((e) => ({ name: e.id, iconItem: e.icon }))
            }

            entries.value = effectList.map((effect) => {
                const shortId = effect.name
                const iconItem = effect.iconItem || 'potion'
                const fullId = `minecraft:${shortId}`

                let display = getSlotDisplay(
                    { kind: 'item', id: `minecraft:${iconItem}` } as unknown as SlotValue,
                    ctx,
                )
                if (!display)
                    display = getSlotDisplay(
                        { kind: 'item', id: 'minecraft:potion' } as unknown as SlotValue,
                        ctx,
                    )

                const displayName = getI18nOrLang(
                    `effect.${shortId}`,
                    `effect.${shortId}`,
                    effect.displayName || shortId,
                )
                return {
                    id: fullId,
                    name: displayName,
                    display: display || undefined,
                    value: { kind: 'item', id: fullId } as SlotValue,
                }
            })
            cachedLocale.value = currentLocale
            return
        }

        // 2. 成就
        if (props.kind === 'advancement') {
            let advList: AdvancementData[] = []
            if (anyRes.advancements) {
                advList = Object.values(anyRes.advancements || {}) as AdvancementData[]
            } else {
                advList = advancementsData
                    .filter((a) => isVersionAtLeast(props.version, a.since))
                    .map((a) => ({ id: a.id, iconItem: a.icon }))
            }

            entries.value = advList.map((adv) => {
                const iconItem = adv.iconItem || adv.display?.icon?.item || 'book'
                const fullId = adv.id.startsWith('minecraft:') ? adv.id : `minecraft:${adv.id}`

                let display = getSlotDisplay(
                    { kind: 'item', id: `minecraft:${iconItem}` } as unknown as SlotValue,
                    ctx,
                )
                if (!display)
                    display = getSlotDisplay(
                        { kind: 'item', id: 'minecraft:book' } as unknown as SlotValue,
                        ctx,
                    )

                const displayName = getI18nOrLang(
                    `advancement.${fullId}.title`,
                    `advancement.${fullId}.title`,
                    fullId.split('/').pop() || fullId,
                )
                return {
                    id: fullId,
                    name: displayName,
                    display: display || undefined,
                    value: { kind: 'item', id: fullId } as SlotValue,
                }
            })
            cachedLocale.value = currentLocale
            return
        }

        // 3. 物品/方块
        let sourceItems: SourceItem[] = resources.items as unknown as SourceItem[]
        let extraBlocks: SourceItem[] = []

        if (props.kind === 'block') {
            sourceItems = (resources.items as unknown as SourceItem[]).filter((item) => {
                return !isNonBlockItem(item.id || item.name || '')
            })

            extraBlocks = EXTRA_BLOCKS_META.map((meta) => {
                return { id: meta.id, __extraMeta: meta }
            })

            sourceItems = [...sourceItems, ...extraBlocks]
        }

        entries.value = sourceItems.map((item) => {
            const value: SlotValue = { kind: 'item', id: item.id }
            let display = undefined

            if (!item.__extraMeta) {
                display = getSlotDisplay(value, ctx)
                if (!display) {
                    display = getSlotDisplay(
                        { kind: 'block', id: item.id } as unknown as SlotValue,
                        ctx,
                    )
                }
            }

            let localizedName = resolveRecipeItemName(
                item.id,
                currentLocale,
                item.name || item.id.replace('minecraft:', ''),
            )

            if (item.__extraMeta) {
                localizedName = formatMessage({
                    id: item.__extraMeta.nameKey,
                    defaultMessage: item.id.split(':')[1],
                })
            }

            return {
                id: item.id,
                name: localizedName,
                display: display || undefined,
                value,
            }
        })
        cachedLocale.value = currentLocale
    } finally {
        loading.value = false
    }
}

function onPick(value: SlotValue) {
    const id = 'id' in value ? value.id : (value as { uid: string }).uid
    if (id) {
        emit('select', id)
        modal.value?.hide()
    }
}

defineExpose({ show })
</script>

<template>
    <NewModal
        ref="modal"
        :header="
            formatMessage(
                kind === 'block'
                    ? messages.titleBlock
                    : kind === 'effect'
                      ? messages.titleEffect
                      : kind === 'advancement'
                        ? messages.titleAdvancement
                        : messages.titleItem,
            )
        "
        width="min(640px, calc(100vw - 2rem))"
        max-width="640px"
        scrollable
        max-content-height="min(40rem, 80vh)"
    >
        <div class="flex h-[32rem] min-h-0 min-w-0 flex-col">
            <CommandGeneratorItemPicker
                :entries="entries"
                :atlas="atlas"
                :loading="loading"
                :kind="kind"
                @pick="onPick"
            />
        </div>
    </NewModal>
</template>
