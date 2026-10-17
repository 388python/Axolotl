<script setup lang="ts">
import { ClipboardCopyIcon, RefreshCwIcon, XIcon } from '@modrinth/assets'
import { Button, Combobox, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, ref, watch } from 'vue'

import { loadMinecraftItemImage } from '@/lab/recipe-generator/item-images'
import {
    getSeedMapContainerLoot,
    getSeedMapStructureDetails,
    type SeedMapLoot,
    type SeedMapStructureDetails,
    type SeedMapStructurePiece,
    type SeedMapStructureQuery,
} from '@/lab/seed-map/advanced'
import type { SeedMapFeatureKind } from '@/lab/seed-map/features'
import { createSeedMapLabels, seedMapEffectDuration } from '@/lab/seed-map/structure-labels'

const props = defineProps<{
    query: SeedMapStructureQuery
    kind: SeedMapFeatureKind
    title: string
}>()
const emit = defineEmits<{ close: []; focus: [piece: SeedMapStructurePiece] }>()
const { formatMessage, locale } = useVIntl()
const messages = defineMessages({
    overview: { id: 'app.lab.seed-map.details.overview', defaultMessage: 'Overview' },
    layout: { id: 'app.lab.seed-map.details.layout', defaultMessage: 'Layout' },
    loot: { id: 'app.lab.seed-map.details.loot', defaultMessage: 'Loot' },
    close: { id: 'app.lab.seed-map.details.close', defaultMessage: 'Close structure details' },
    retry: { id: 'app.lab.seed-map.details.retry', defaultMessage: 'Retry' },
    loading: { id: 'app.lab.seed-map.details.loading', defaultMessage: 'Calculating structure…' },
    failed: {
        id: 'app.lab.seed-map.details.failed',
        defaultMessage: 'The calculation failed. Your selection is preserved.',
    },
    unsupported: {
        id: 'app.lab.seed-map.details.unsupported',
        defaultMessage: 'The engine has no additional details for this structure in this version.',
    },
    rotation: { id: 'app.lab.seed-map.details.rotation', defaultMessage: 'Rotation' },
    mirror: { id: 'app.lab.seed-map.details.mirror', defaultMessage: 'Mirror' },
    variant: { id: 'app.lab.seed-map.details.variant', defaultMessage: 'Variant' },
    abandoned: { id: 'app.lab.seed-map.details.abandoned', defaultMessage: 'Abandoned village' },
    giant: { id: 'app.lab.seed-map.details.giant', defaultMessage: 'Giant portal' },
    underground: { id: 'app.lab.seed-map.details.underground', defaultMessage: 'Underground' },
    airPocket: { id: 'app.lab.seed-map.details.air-pocket', defaultMessage: 'Air pocket' },
    basement: { id: 'app.lab.seed-map.details.basement', defaultMessage: 'Basement' },
    cracked: { id: 'app.lab.seed-map.details.cracked', defaultMessage: 'Cracked geode' },
    ship: { id: 'app.lab.seed-map.details.ship', defaultMessage: 'End ship' },
    eyes: { id: 'app.lab.seed-map.details.eyes', defaultMessage: '{count} portal eyes' },
    pieces: { id: 'app.lab.seed-map.details.pieces', defaultMessage: '{count} structure pieces' },
    containers: { id: 'app.lab.seed-map.details.containers', defaultMessage: '{count} containers' },
    container: {
        id: 'app.lab.seed-map.details.named-container',
        defaultMessage: '{name} {number} · {x}, {z}',
    },
    technical: {
        id: 'app.lab.seed-map.details.technical-identifiers',
        defaultMessage: 'Technical identifiers',
    },
    effectDuration: {
        id: 'app.lab.seed-map.details.effect-duration',
        defaultMessage: '{name} · {duration}',
    },
    enchanted: {
        id: 'app.lab.seed-map.details.enchantment-level',
        defaultMessage: '{name} {level}',
    },
    noLayout: {
        id: 'app.lab.seed-map.details.no-layout',
        defaultMessage: 'Internal layout is not simulated by this engine.',
    },
    noLoot: {
        id: 'app.lab.seed-map.details.no-loot',
        defaultMessage:
            'No predictable containers are available for this structure in this version.',
    },
    predictedLoot: {
        id: 'app.lab.seed-map.details.predicted-loot',
        defaultMessage: 'Predicted item list; chest slot positions are not reconstructed.',
    },
    unresolved: {
        id: 'app.lab.seed-map.details.unresolved',
        defaultMessage: 'Some loot subtables are missing; this list is incomplete.',
    },
    copy: { id: 'app.lab.seed-map.details.copy', defaultMessage: 'Copy predicted loot' },
    copied: { id: 'app.lab.seed-map.details.copied', defaultMessage: 'Copied' },
    height: {
        id: 'app.lab.seed-map.details.height',
        defaultMessage:
            'Some Y coordinates are placeholders. Use the horizontal layout to locate pieces.',
    },
    partial: {
        id: 'app.lab.seed-map.details.partial',
        defaultMessage: 'The engine simulates only part of this structure.',
    },
    terrain: {
        id: 'app.lab.seed-map.details.terrain',
        defaultMessage: 'Terrain and later world changes can affect this prediction.',
    },
    truncated: {
        id: 'app.lab.seed-map.details.truncated',
        defaultMessage: 'The structure reached the piece limit; additional pieces may be missing.',
    },
    estimatedContainers: {
        id: 'app.lab.seed-map.details.estimated-containers',
        defaultMessage:
            'Container coordinates are rough chunk estimates; exact chest positions are unavailable.',
    },
    positionsUnavailable: {
        id: 'app.lab.seed-map.details.positions-unavailable',
        defaultMessage:
            'The engine provides container locations but no piece positions for this structure.',
    },
    layoutMap: {
        id: 'app.lab.seed-map.details.layout-map',
        defaultMessage: 'Structure layout. Select a piece to highlight it on the map.',
    },
})

const tab = ref<'overview' | 'layout' | 'loot'>('overview')
const details = ref<SeedMapStructureDetails | null>(null)
const loot = ref<SeedMapLoot | null>(null)
const busy = ref(false)
const lootBusy = ref(false)
const failed = ref(false)
const lootFailed = ref(false)
const containerId = ref<number>()
const selectedPiece = ref<number>()
const copied = ref(false)
const technicalOpen = ref(false)
const labels = computed(() =>
    createSeedMapLabels((message, values) => formatMessage(message, values), locale.value),
)
const variantLabel = computed(() =>
    details.value?.variant
        ? labels.value.variant(props.kind, details.value.variant, details.value.pieces)
        : undefined,
)
const technicalIdentifiers = computed(() =>
    JSON.stringify(
        {
            kind: props.kind,
            variant: details.value?.variant,
            pieces: details.value?.pieces.map((piece) => ({
                id: piece.id,
                engineName: piece.name,
                identifier: labels.value.pieceId(piece.name, details.value?.variant),
            })),
            containers: details.value?.containers,
            loot: loot.value,
        },
        null,
        2,
    ),
)
const itemImages = ref<Record<string, string>>({})
const lootCache = new Map<number, SeedMapLoot>()
let generation = 0
let lootGeneration = 0

async function loadDetails() {
    const token = ++generation
    const query = { ...props.query }
    busy.value = true
    failed.value = false
    try {
        const result = await getSeedMapStructureDetails(query)
        if (token !== generation) return
        details.value = result
        containerId.value = result.containers[0]?.id
        if (tab.value === 'loot') void loadLoot()
    } catch {
        if (token === generation) failed.value = true
    } finally {
        if (token === generation) busy.value = false
    }
}

async function loadLoot() {
    if (containerId.value === undefined || tab.value !== 'loot') return
    const token = ++lootGeneration
    const container = containerId.value
    const query = { ...props.query }
    lootBusy.value = true
    lootFailed.value = false
    copied.value = false
    try {
        const result = lootCache.get(container) ?? (await getSeedMapContainerLoot(query, container))
        if (token !== lootGeneration) return
        loot.value = result
        if (lootCache.size >= 32) lootCache.delete(lootCache.keys().next().value!)
        lootCache.set(container, result)
        for (const item of result.items ?? []) {
            if (itemImages.value[item.id]) continue
            void loadMinecraftItemImage(item.id)
                .then((image) => {
                    itemImages.value[item.id] = image.src
                })
                .catch(() => {})
        }
    } catch {
        if (token === lootGeneration) lootFailed.value = true
    } finally {
        if (token === lootGeneration) lootBusy.value = false
    }
}

watch(
    () => JSON.stringify(props.query),
    () => {
        generation++
        lootGeneration++
        details.value = null
        loot.value = null
        selectedPiece.value = undefined
        technicalOpen.value = false
        lootCache.clear()
        void loadDetails()
    },
    { immediate: true },
)
watch([tab, containerId], () => {
    loot.value = null
    void loadLoot()
})

const positionedPieces = computed(
    () => details.value?.pieces.filter((piece) => piece.positionKnown) ?? [],
)
const bounds = computed(() => {
    const pieces = positionedPieces.value
    const minX = Math.min(...pieces.map((piece) => piece.min?.x ?? piece.position.x), props.query.x)
    const minZ = Math.min(...pieces.map((piece) => piece.min?.z ?? piece.position.z), props.query.z)
    const maxX = Math.max(
        ...pieces.map((piece) => piece.max?.x ?? piece.position.x),
        props.query.x + 1,
    )
    const maxZ = Math.max(
        ...pieces.map((piece) => piece.max?.z ?? piece.position.z),
        props.query.z + 1,
    )
    return `${minX - 4} ${minZ - 4} ${maxX - minX + 8} ${maxZ - minZ + 8}`
})
const flags = computed(() => {
    const variant = details.value?.variant
    if (!variant) return []
    const values = {
        abandoned: props.kind === 'village' && variant.abandoned,
        giant: props.kind === 'ruined-portal' && variant.giant,
        underground: props.kind === 'ruined-portal' && variant.underground,
        airPocket: props.kind === 'ruined-portal' && variant.airPocket,
        basement: props.kind === 'igloo' && variant.basement,
        cracked: props.kind === 'geode' && variant.cracked,
    }
    return (Object.keys(values) as (keyof typeof values)[]).filter((key) => values[key])
})
const portalEyes = computed(
    () => details.value?.pieces.find((piece) => piece.portalEyes !== undefined)?.portalEyes,
)
const hasShip = computed(() => details.value?.pieces.some((piece) => piece.name.includes('ship')))
function pieceLabel(piece: SeedMapStructurePiece) {
    return labels.value.piece(props.kind, piece.name, details.value?.variant)
}
function effectLabel(effect: string, duration?: number) {
    const name = labels.value.effect(effect)
    const instant = ['instant_health', 'instant_damage', 'saturation'].includes(
        effect.replace(/^minecraft:/, ''),
    )
    const time = duration === undefined || instant ? undefined : seedMapEffectDuration(duration)
    return time ? formatMessage(messages.effectDuration, { name, duration: time }) : name
}
function focus(piece: SeedMapStructurePiece) {
    if (!piece.positionKnown) return
    selectedPiece.value = piece.id
    emit('focus', piece)
}
onBeforeUnmount(() => {
    generation++
    lootGeneration++
})
async function copyLoot() {
    try {
        await navigator.clipboard.writeText(JSON.stringify(loot.value, null, 2))
        copied.value = true
    } catch {
        lootFailed.value = true
    }
}
</script>

<template>
    <aside
        class="absolute inset-y-3 right-3 z-20 flex w-[min(26rem,calc(100%_-_1.5rem))] min-h-0 flex-col overflow-hidden rounded-xl border border-surface-4 bg-surface-2 shadow-lg"
        :aria-label="title"
        @pointerdown.stop
        @wheel.stop
        @dblclick.stop
    >
        <div
            class="flex shrink-0 items-center justify-between gap-3 border-b border-surface-4 px-4 py-3"
        >
            <div class="min-w-0">
                <strong class="block truncate">{{ title }}</strong
                ><span class="text-xs text-[var(--color-text-tertiary)]"
                    >{{ query.version }} · {{ query.x }}, {{ query.z }}</span
                >
            </div>
            <Button
                type="quiet"
                size="2xs"
                circular
                icon-only
                :aria-label="formatMessage(messages.close)"
                @click="emit('close')"
                ><XIcon
            /></Button>
        </div>
        <div class="flex shrink-0 gap-1 border-b border-surface-4 p-2" role="tablist">
            <Button
                v-for="value in ['overview', 'layout', 'loot'] as const"
                :key="value"
                :type="tab === value ? 'chip-text' : 'quiet'"
                size="2xs"
                role="tab"
                :aria-selected="tab === value"
                @click="tab = value"
                >{{ formatMessage(messages[value]) }}</Button
            >
        </div>
        <div
            class="min-h-0 flex-1 overflow-y-auto p-4"
            :aria-busy="busy || lootBusy"
            data-onboarding-id="seed-map-structure-details"
        >
            <p v-if="busy" class="m-0 text-sm">{{ formatMessage(messages.loading) }}</p>
            <div v-if="failed" class="flex flex-col gap-2 text-sm">
                <p class="m-0">{{ formatMessage(messages.failed) }}</p>
                <Button type="outlined" size="2xs" @click="loadDetails"
                    ><RefreshCwIcon />{{ formatMessage(messages.retry) }}</Button
                >
            </div>
            <div class="mb-3 flex flex-col gap-2"><slot name="actions" /></div>
            <template v-if="details">
                <p v-if="!details.supported" class="m-0 text-sm">
                    {{ formatMessage(messages.unsupported) }}
                </p>
                <div v-if="tab === 'overview'" class="flex flex-col gap-3 text-sm">
                    <dl v-if="details.variant" class="m-0 grid grid-cols-2 gap-2">
                        <template v-if="labels.hasRotation(kind)">
                            <dt>{{ formatMessage(messages.rotation) }}</dt>
                            <dd class="m-0">{{ labels.rotation(details.variant.rotation) }}</dd>
                        </template>
                        <template v-if="labels.hasMirror(kind)">
                            <dt>{{ formatMessage(messages.mirror) }}</dt>
                            <dd class="m-0">{{ labels.mirror(details.variant.mirror) }}</dd>
                        </template>
                        <template v-if="variantLabel">
                            <dt>{{ formatMessage(messages.variant) }}</dt>
                            <dd class="m-0 min-w-0">
                                {{ variantLabel.name }}
                                <code
                                    v-if="variantLabel.id"
                                    class="block break-all text-xs text-[var(--color-text-tertiary)]"
                                    >{{ variantLabel.id }}</code
                                >
                            </dd>
                        </template>
                    </dl>
                    <span v-for="flag in flags" :key="flag">{{
                        formatMessage(messages[flag])
                    }}</span>
                    <span v-if="kind === 'end-city' && hasShip">{{
                        formatMessage(messages.ship)
                    }}</span>
                    <span v-if="portalEyes !== undefined">{{
                        formatMessage(messages.eyes, { count: portalEyes })
                    }}</span>
                    <span>{{
                        formatMessage(messages.pieces, { count: details.pieces.length })
                    }}</span>
                    <span>{{
                        formatMessage(messages.containers, { count: details.containers.length })
                    }}</span>
                </div>
                <div v-else-if="tab === 'layout'" class="flex flex-col gap-2">
                    <p v-if="!details.pieces.length" class="m-0 text-sm">
                        {{ formatMessage(messages.noLayout) }}
                    </p>
                    <svg
                        v-if="positionedPieces.length"
                        :viewBox="bounds"
                        class="h-52 w-full rounded-lg border border-surface-4 bg-surface-1 text-[var(--color-brand)]"
                        :aria-label="formatMessage(messages.layoutMap)"
                    >
                        <rect
                            v-for="piece in positionedPieces"
                            :key="piece.id"
                            :x="piece.min?.x ?? piece.position.x"
                            :y="piece.min?.z ?? piece.position.z"
                            :width="
                                Math.max(
                                    1,
                                    (piece.max?.x ?? piece.position.x) -
                                        (piece.min?.x ?? piece.position.x) +
                                        1,
                                )
                            "
                            :height="
                                Math.max(
                                    1,
                                    (piece.max?.z ?? piece.position.z) -
                                        (piece.min?.z ?? piece.position.z) +
                                        1,
                                )
                            "
                            fill="currentColor"
                            :fill-opacity="selectedPiece === piece.id ? 0.8 : 0.25"
                            stroke="currentColor"
                            stroke-width="0.5"
                            role="button"
                            tabindex="0"
                            :aria-label="pieceLabel(piece)"
                            @click="focus(piece)"
                            @keydown.enter="focus(piece)"
                        />
                    </svg>
                    <button
                        v-for="piece in details.pieces"
                        :key="piece.id"
                        class="flex flex-col gap-1 rounded-lg border border-surface-4 bg-surface-3 p-2 text-left text-xs hover:bg-surface-4"
                        @click="focus(piece)"
                    >
                        <strong class="break-words">{{ pieceLabel(piece) }}</strong
                        ><span v-if="piece.positionKnown"
                            >{{ piece.position.x }}, {{ piece.position.z }} ·
                            {{ labels.rotation(piece.rotation) }}</span
                        ><span v-if="piece.portalEyes !== undefined">{{
                            formatMessage(messages.eyes, { count: piece.portalEyes })
                        }}</span>
                    </button>
                </div>
                <div
                    v-else
                    class="flex flex-col gap-3 text-sm"
                    data-onboarding-id="seed-map-container-loot"
                >
                    <p class="m-0 text-xs text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.predictedLoot) }}
                    </p>
                    <p v-if="!details.containers.length" class="m-0">
                        {{ formatMessage(messages.noLoot) }}
                    </p>
                    <Combobox
                        v-else
                        :model-value="String(containerId)"
                        :options="
                            details.containers.map((container, index) => ({
                                value: String(container.id),
                                label: formatMessage(messages.container, {
                                    number: index + 1,
                                    name: labels.table(container.table),
                                    x: container.x,
                                    z: container.z,
                                }),
                            }))
                        "
                        @update:model-value="containerId = Number($event)"
                    />
                    <p v-if="lootBusy" class="m-0">{{ formatMessage(messages.loading) }}</p>
                    <div v-if="lootFailed" class="flex flex-col gap-2">
                        <p class="m-0">{{ formatMessage(messages.failed) }}</p>
                        <Button type="outlined" size="2xs" @click="loadLoot">{{
                            formatMessage(messages.retry)
                        }}</Button>
                    </div>
                    <template v-if="loot">
                        <p v-if="!loot.supported" class="m-0">
                            {{ formatMessage(messages.noLoot) }}
                        </p>
                        <p v-if="loot.unresolvedTables?.length" class="m-0">
                            {{ formatMessage(messages.unresolved)
                            }}<span
                                v-for="table in loot.unresolvedTables"
                                :key="table"
                                class="block text-xs"
                                >{{ labels.table(table) }}</span
                            >
                        </p>
                        <div
                            v-for="(item, index) in loot.items"
                            :key="index"
                            class="flex gap-2 border-b border-surface-4 py-2"
                        >
                            <img
                                v-if="itemImages[item.id]"
                                :src="itemImages[item.id]"
                                alt=""
                                class="size-8 shrink-0 [image-rendering:pixelated]"
                            />
                            <div class="min-w-0">
                                <strong class="break-words"
                                    >{{ labels.item(item.id) }} ×{{ item.count }}</strong
                                ><span
                                    v-for="enchantment in item.enchantments"
                                    :key="enchantment.id"
                                    class="block text-xs text-[var(--color-text-tertiary)]"
                                    >{{
                                        formatMessage(messages.enchanted, {
                                            name: labels.enchantment(enchantment.id),
                                            level: labels.level(enchantment.level),
                                        })
                                    }}</span
                                ><span v-if="item.effect" class="block text-xs">{{
                                    effectLabel(item.effect, item.duration)
                                }}</span>
                            </div>
                        </div>
                        <Button v-if="loot.supported" type="outlined" size="2xs" @click="copyLoot"
                            ><ClipboardCopyIcon />{{
                                formatMessage(copied ? messages.copied : messages.copy)
                            }}</Button
                        >
                    </template>
                </div>
                <div
                    class="mt-4 flex flex-col gap-2 border-t border-surface-4 pt-3 text-xs text-[var(--color-text-tertiary)]"
                >
                    <p v-if="details.limits.includes('height_placeholder')" class="m-0">
                        {{ formatMessage(messages.height) }}
                    </p>
                    <p v-if="details.limits.includes('partial_layout')" class="m-0">
                        {{ formatMessage(messages.partial) }}
                    </p>
                    <p v-if="details.limits.includes('terrain_unverified')" class="m-0">
                        {{ formatMessage(messages.terrain) }}
                    </p>
                    <p v-if="details.limits.includes('piece_positions_unavailable')" class="m-0">
                        {{ formatMessage(messages.positionsUnavailable) }}
                    </p>
                    <p
                        v-if="details.limits.includes('container_positions_approximate')"
                        class="m-0"
                    >
                        {{ formatMessage(messages.estimatedContainers) }}
                    </p>
                    <p v-if="details.truncated" class="m-0">
                        {{ formatMessage(messages.truncated) }}
                    </p>
                </div>
                <details
                    :open="technicalOpen"
                    class="mt-3 text-xs text-[var(--color-text-tertiary)]"
                    @toggle="technicalOpen = ($event.target as HTMLDetailsElement).open"
                >
                    <summary class="cursor-pointer">
                        {{ formatMessage(messages.technical) }}
                    </summary>
                    <pre v-if="technicalOpen" class="m-0 mt-2 whitespace-pre-wrap break-all">{{
                        technicalIdentifiers
                    }}</pre>
                </details>
            </template>
        </div>
    </aside>
</template>
