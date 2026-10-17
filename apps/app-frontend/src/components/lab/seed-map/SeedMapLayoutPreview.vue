<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import type { SeedMapStructurePiece } from '@/lab/seed-map/advanced'
import type { SeedMapLayoutScene } from '@/lab/seed-map/layout-scene'
import type { PreviewAxis } from '@/lab/seed-map/preview-overlays'
import {
    type StructureBlocksResponse,
    type StructureBlocksResult,
    type StructureChest,
    supportsStructureBlocks,
} from '@/lab/seed-map/structure-blocks'

import SeedMapOrientation from './SeedMapOrientation.vue'

const props = defineProps<{
    pieces: SeedMapStructurePiece[]
    selected?: number
    label: (piece: SeedMapStructurePiece) => string
    version?: string
    kind?: string
    chests?: StructureChest[]
    unknownChestHeights?: number
}>()
const emit = defineEmits<{ select: [piece: SeedMapStructurePiece] }>()
const { formatMessage } = useVIntl()
const messages = defineMessages({
    title: { id: 'app.lab.seed-map.preview.title', defaultMessage: '3D layout preview' },
    controls: {
        id: 'app.lab.seed-map.preview.controls',
        defaultMessage: 'Drag to rotate · Scroll to zoom · Right-drag to pan',
    },
    fit: { id: 'app.lab.seed-map.preview.fit', defaultMessage: 'Fit structure' },
    failed: {
        id: 'app.lab.seed-map.preview.failed',
        defaultMessage: '3D rendering is unavailable. The horizontal layout is shown below.',
    },
    retry: { id: 'app.lab.seed-map.preview.retry', defaultMessage: 'Retry 3D preview' },
    bounds: {
        id: 'app.lab.seed-map.preview.bounds',
        defaultMessage: 'Structure piece bounds; individual blocks are not simulated.',
    },
    frames: { id: 'app.lab.seed-map.preview.frames', defaultMessage: 'Structure frames' },
    blocks: { id: 'app.lab.seed-map.preview.blocks', defaultMessage: 'Real structure' },
    loadingBlocks: {
        id: 'app.lab.seed-map.preview.loading-blocks',
        defaultMessage: 'Building structure…',
    },
    blocksFailed: {
        id: 'app.lab.seed-map.preview.blocks-failed',
        defaultMessage: 'Could not load the structure. Frames are still available.',
    },
    retryBlocks: { id: 'app.lab.seed-map.preview.retry-blocks', defaultMessage: 'Retry structure' },
    templateNote: {
        id: 'app.lab.seed-map.preview.template-note',
        defaultMessage:
            'Original {version} template blocks colored from their textures. Heights follow the predicted layout; entities are not shown.',
    },
    missingModels: {
        id: 'app.lab.seed-map.preview.missing-models',
        defaultMessage:
            '{count, plural, one {# block type has no preview model and is omitted.} other {# block types have no preview model and are omitted.}}',
    },
    shipwreckLimit: {
        id: 'app.lab.seed-map.preview.shipwreck-limit',
        defaultMessage:
            'Template shape is predicted. A representative original palette is shown; the engine does not return the selected wood palette.',
    },
    outpostLimit: {
        id: 'app.lab.seed-map.preview.outpost-limit',
        defaultMessage:
            'Only the main watchtower is shown. Overgrowth, foundations and surrounding buildings are not reconstructed.',
    },
    portalLimit: {
        id: 'app.lab.seed-map.preview.portal-limit',
        defaultMessage:
            'Portal template only. Weathering, random block replacements and surrounding terrain are not reconstructed; Y is a placeholder.',
    },
    chestXray: {
        id: 'app.lab.seed-map.preview.chest-xray',
        defaultMessage: 'Chest X-ray ({count})',
    },
    chestLoading: {
        id: 'app.lab.seed-map.preview.chest-loading',
        defaultMessage: 'Locating chests…',
    },
    chestFailed: {
        id: 'app.lab.seed-map.preview.chest-failed',
        defaultMessage: 'Chest positions could not be loaded.',
    },
    chestRetry: {
        id: 'app.lab.seed-map.preview.chest-retry',
        defaultMessage: 'Retry chest positions',
    },
    unknownChestHeights: {
        id: 'app.lab.seed-map.preview.chest-height-unknown',
        defaultMessage:
            '{count, plural, one {# container has no known Y coordinate and is not highlighted in 3D.} other {# containers have no known Y coordinates and are not highlighted in 3D.}}',
    },
})
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')
const failed = ref(false)
const ready = ref(false)
const axes = ref<PreviewAxis[]>([])
const chestPositions = ref<StructureChest[]>(props.chests ?? [])
const chestXray = ref(true)
const chestBusy = ref(false)
const chestFailed = ref(false)
let chestWorker: Worker | undefined
let chestTimeout: ReturnType<typeof setTimeout> | undefined
let chestGeneration = 0
let chestsLoaded = false
const blocksEnabled = ref(false)
const blocksBusy = ref(false)
const blocksFailed = ref(false)
const missingModels = ref(0)
const blocksAvailable = computed(() =>
    supportsStructureBlocks(props.version ?? '', props.kind ?? ''),
)
const templateLimit = computed(() =>
    props.kind === 'shipwreck'
        ? messages.shipwreckLimit
        : props.kind === 'outpost'
          ? messages.outpostLimit
          : props.kind === 'ruined-portal'
            ? messages.portalLimit
            : undefined,
)
let blockResult: StructureBlocksResult | undefined
let worker: Worker | undefined
let timeout: ReturnType<typeof setTimeout> | undefined
let blockGeneration = 0
let scene: SeedMapLayoutScene | undefined
let generation = 0
function stopChestWork() {
    chestGeneration++
    chestWorker?.terminate()
    chestWorker = undefined
    clearTimeout(chestTimeout)
    chestBusy.value = false
}
function applyChests(chests: StructureChest[]) {
    chestFailed.value = false
    chestPositions.value = chests
    chestsLoaded = true
    scene?.setChests(chests)
    scene?.showChests(chestXray.value)
}
function loadChests() {
    if (!scene || !blocksAvailable.value || chestsLoaded || chestBusy.value) return
    stopChestWork()
    const token = chestGeneration
    chestBusy.value = true
    chestFailed.value = false
    const fail = () => {
        if (token !== chestGeneration) return
        stopChestWork()
        chestFailed.value = true
    }
    try {
        chestWorker = new Worker(
            new URL('../../../lab/seed-map/structure-block-worker.ts', import.meta.url),
            { type: 'module' },
        )
        chestWorker.onerror = fail
        chestWorker.onmessage = (event: MessageEvent<StructureBlocksResponse>) => {
            if (token !== chestGeneration) return
            if (event.data.type !== 'chests') {
                fail()
                return
            }
            applyChests(event.data.chests)
            stopChestWork()
        }
        chestTimeout = setTimeout(fail, 15_000)
        chestWorker.postMessage({
            mode: 'chests',
            version: props.version,
            pieces: JSON.parse(JSON.stringify(props.pieces)),
        })
    } catch {
        fail()
    }
}
function stopBlockWork() {
    blockGeneration++
    worker?.terminate()
    worker = undefined
    clearTimeout(timeout)
    timeout = undefined
    blocksBusy.value = false
}
function loadBlocks() {
    if (!scene || !blocksAvailable.value || blocksBusy.value) return
    if (blockResult) {
        scene.showBlocks(true)
        return
    }
    stopBlockWork()
    stopChestWork()
    chestFailed.value = false
    const token = blockGeneration
    blocksBusy.value = true
    blocksFailed.value = false
    const fail = () => {
        if (token !== blockGeneration) return
        stopBlockWork()
        blocksFailed.value = true
        blocksEnabled.value = false
        scene?.showBlocks(false)
    }
    try {
        worker = new Worker(
            new URL('../../../lab/seed-map/structure-block-worker.ts', import.meta.url),
            { type: 'module' },
        )
        worker.onerror = fail
        worker.onmessage = (event: MessageEvent<StructureBlocksResponse>) => {
            if (token !== blockGeneration) return
            if (event.data.type !== 'ready') {
                fail()
                return
            }
            try {
                blockResult = event.data.result
                scene?.setBlocks(blockResult)
                applyChests(blockResult.chests)
                missingModels.value = blockResult.missingModels.length
                scene?.showBlocks(blocksEnabled.value)
                stopBlockWork()
            } catch {
                blockResult = undefined
                fail()
            }
        }
        timeout = setTimeout(fail, 30_000)
        worker.postMessage({
            version: props.version,
            pieces: JSON.parse(JSON.stringify(props.pieces)),
        })
    } catch {
        fail()
    }
}
function setBlockMode(show: boolean) {
    blocksEnabled.value = show
    blocksFailed.value = false
    if (show) loadBlocks()
    else {
        stopBlockWork()
        scene?.showBlocks(false)
        loadChests()
    }
}
async function initialize() {
    const token = ++generation
    stopBlockWork()
    stopChestWork()
    scene?.dispose()
    scene = undefined
    failed.value = false
    ready.value = false
    try {
        const { SeedMapLayoutScene } = await import('@/lab/seed-map/layout-scene')
        if (token !== generation || !canvas.value) return
        scene = new SeedMapLayoutScene(
            canvas.value,
            props.pieces,
            (piece) => emit('select', piece),
            (value) => {
                axes.value = value
            },
        )
        scene.select(props.selected)
        scene.setChests(chestPositions.value)
        scene.showChests(chestXray.value)
        if (blockResult) scene.setBlocks(blockResult)
        ready.value = true
        if (blocksEnabled.value) loadBlocks()
        else loadChests()
    } catch {
        if (token === generation) failed.value = true
    }
}
function contextLost(event: Event) {
    event.preventDefault()
    generation++
    stopBlockWork()
    stopChestWork()
    scene?.dispose()
    scene = undefined
    failed.value = true
    ready.value = false
}
const bounds = computed(() => {
    const minX = Math.min(...props.pieces.map((piece) => piece.min!.x))
    const minZ = Math.min(...props.pieces.map((piece) => piece.min!.z))
    const maxX = Math.max(...props.pieces.map((piece) => piece.max!.x + 1))
    const maxZ = Math.max(...props.pieces.map((piece) => piece.max!.z + 1))
    return `${minX - 4} ${minZ - 4} ${maxX - minX + 8} ${maxZ - minZ + 8}`
})
onMounted(initialize)
watch([() => props.pieces, () => props.version, () => props.kind, () => props.chests], () => {
    blockResult = undefined
    blocksEnabled.value = false
    blocksFailed.value = false
    missingModels.value = 0
    chestPositions.value = props.chests ?? []
    chestsLoaded = false
    chestFailed.value = false
    initialize()
})
watch(chestXray, (value) => scene?.showChests(value))
watch(
    () => props.selected,
    (id) => scene?.select(id),
)
onBeforeUnmount(() => {
    generation++
    stopBlockWork()
    stopChestWork()
    scene?.dispose()
})
</script>

<template>
    <div class="flex flex-col gap-2" data-testid="seed-map-layout-preview">
        <div class="flex items-center justify-between gap-2 text-sm">
            <strong>{{ formatMessage(messages.title) }}</strong>
            <Button v-if="!failed" type="outlined" size="2xs" @click="scene?.fit()">{{
                formatMessage(messages.fit)
            }}</Button>
        </div>
        <div
            v-if="!failed && (blocksAvailable || kind === 'fortress' || chestPositions.length)"
            class="flex flex-wrap items-center gap-2 text-xs"
        >
            <Button
                size="2xs"
                :type="chestXray ? 'base' : 'outlined'"
                :aria-pressed="chestXray"
                :disabled="!ready"
                data-testid="seed-map-chest-xray"
                @click="chestXray = !chestXray"
                >{{ formatMessage(messages.chestXray, { count: chestPositions.length }) }}</Button
            >
            <span v-if="chestBusy" role="status">{{ formatMessage(messages.chestLoading) }}</span>
        </div>
        <div
            v-if="blocksAvailable && !failed"
            class="flex flex-wrap gap-1"
            role="group"
            :aria-label="formatMessage(messages.title)"
        >
            <Button
                size="2xs"
                :type="!blocksEnabled ? 'base' : 'outlined'"
                :aria-pressed="!blocksEnabled"
                data-testid="seed-map-frames-mode"
                @click="setBlockMode(false)"
                >{{ formatMessage(messages.frames) }}</Button
            >
            <Button
                size="2xs"
                :type="blocksEnabled ? 'base' : 'outlined'"
                :aria-pressed="blocksEnabled"
                :disabled="!ready"
                data-testid="seed-map-blocks-mode"
                @click="setBlockMode(true)"
                >{{ formatMessage(messages.blocks) }}</Button
            >
        </div>
        <div
            class="relative aspect-[4/3] min-h-[14rem] overflow-hidden rounded-lg border border-surface-4 bg-surface-1"
        >
            <canvas
                v-show="!failed"
                ref="canvas"
                class="absolute inset-0 size-full touch-none"
                :data-ready="ready"
                :data-chest-count="chestPositions.length"
                :data-chest-xray="chestXray"
                :data-render-mode="
                    blocksEnabled && blockResult && !blocksBusy ? 'blocks' : 'frames'
                "
                :aria-label="formatMessage(messages.title)"
                @webglcontextlost="contextLost"
                @webglcontextrestored="initialize"
                @contextmenu.prevent
            />
            <SeedMapOrientation v-if="!failed && ready" :axes="axes" />
            <span
                v-if="blocksBusy"
                role="status"
                class="absolute left-2 top-2 rounded-md border border-surface-4 bg-surface-2 px-2 py-1 text-xs"
                >{{ formatMessage(messages.loadingBlocks) }}</span
            >
            <svg
                v-if="failed"
                :viewBox="bounds"
                class="absolute inset-0 size-full text-[var(--color-brand)]"
                :aria-label="formatMessage(messages.title)"
            >
                <rect
                    v-for="piece in pieces"
                    :key="piece.id"
                    :x="piece.min!.x"
                    :y="piece.min!.z"
                    :width="piece.max!.x - piece.min!.x + 1"
                    :height="piece.max!.z - piece.min!.z + 1"
                    fill="currentColor"
                    :fill-opacity="selected === piece.id ? 0.7 : 0.2"
                    stroke="currentColor"
                    stroke-width="0.5"
                    role="button"
                    tabindex="0"
                    :aria-label="label(piece)"
                    @click="emit('select', piece)"
                    @keydown.enter="emit('select', piece)"
                />
            </svg>
        </div>
        <p class="m-0 text-xs text-[var(--color-text-tertiary)]">
            {{ formatMessage(failed ? messages.failed : messages.controls) }}
        </p>
        <div v-if="chestFailed" role="alert" class="flex items-center gap-2 text-xs">
            <span>{{ formatMessage(messages.chestFailed) }}</span>
            <Button size="2xs" type="outlined" @click="loadChests">{{
                formatMessage(messages.chestRetry)
            }}</Button>
        </div>
        <p v-if="unknownChestHeights" class="m-0 text-xs text-[var(--color-text-tertiary)]">
            {{ formatMessage(messages.unknownChestHeights, { count: unknownChestHeights }) }}
        </p>
        <p
            v-if="blocksEnabled && !blocksBusy"
            class="m-0 text-xs text-[var(--color-text-tertiary)]"
        >
            {{ formatMessage(messages.templateNote, { version: props.version ?? '' }) }}
        </p>
        <p
            v-if="blocksEnabled && missingModels"
            class="m-0 text-xs text-[var(--color-text-tertiary)]"
        >
            {{ formatMessage(messages.missingModels, { count: missingModels }) }}
        </p>
        <p
            v-if="blocksEnabled && templateLimit"
            class="m-0 text-xs text-[var(--color-text-tertiary)]"
        >
            {{ formatMessage(templateLimit) }}
        </p>
        <div v-if="blocksFailed" role="alert" class="flex flex-col items-start gap-2 text-xs">
            <span>{{ formatMessage(messages.blocksFailed) }}</span>
            <Button size="2xs" type="outlined" @click="setBlockMode(true)">{{
                formatMessage(messages.retryBlocks)
            }}</Button>
        </div>
        <Button v-if="failed" type="outlined" size="2xs" @click="initialize">{{
            formatMessage(messages.retry)
        }}</Button>
    </div>
</template>
