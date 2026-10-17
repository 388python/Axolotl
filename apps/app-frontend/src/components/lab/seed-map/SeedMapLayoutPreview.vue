<script setup lang="ts">
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import type { SeedMapStructurePiece } from '@/lab/seed-map/advanced'
import type { SeedMapLayoutScene } from '@/lab/seed-map/layout-scene'

const props = defineProps<{
    pieces: SeedMapStructurePiece[]
    selected?: number
    label: (piece: SeedMapStructurePiece) => string
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
})
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')
const failed = ref(false)
const ready = ref(false)
let scene: SeedMapLayoutScene | undefined
let generation = 0
async function initialize() {
    const token = ++generation
    scene?.dispose()
    scene = undefined
    failed.value = false
    ready.value = false
    try {
        const { SeedMapLayoutScene } = await import('@/lab/seed-map/layout-scene')
        if (token !== generation || !canvas.value) return
        scene = new SeedMapLayoutScene(canvas.value, props.pieces, (piece) => emit('select', piece))
        scene.select(props.selected)
        ready.value = true
    } catch {
        if (token === generation) failed.value = true
    }
}
function contextLost(event: Event) {
    event.preventDefault()
    generation++
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
watch(() => props.pieces, initialize)
watch(
    () => props.selected,
    (id) => scene?.select(id),
)
onBeforeUnmount(() => {
    generation++
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
            class="relative aspect-[4/3] min-h-[14rem] overflow-hidden rounded-lg border border-surface-4 bg-surface-1"
        >
            <canvas
                v-show="!failed"
                ref="canvas"
                class="absolute inset-0 size-full touch-none"
                :data-ready="ready"
                :aria-label="formatMessage(messages.title)"
                @webglcontextlost="contextLost"
                @webglcontextrestored="initialize"
                @contextmenu.prevent
            />
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
        <p class="m-0 text-xs text-[var(--color-text-tertiary)]">
            {{ formatMessage(messages.bounds) }}
        </p>
        <Button v-if="failed" type="outlined" size="2xs" @click="initialize">{{
            formatMessage(messages.retry)
        }}</Button>
    </div>
</template>
