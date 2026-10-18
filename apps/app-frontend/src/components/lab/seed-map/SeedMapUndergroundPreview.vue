<script setup lang="ts">
import { ArrowLeftIcon, RefreshCwIcon } from '@modrinth/assets'
import { Button, defineMessages, Toggle, useVIntl } from '@modrinth/ui'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { platform } from '@tauri-apps/plugin-os'
import type { CanvasTexture } from 'three'
import { computed, onBeforeUnmount, onMounted, reactive, ref, useTemplateRef, watch } from 'vue'

import type { SchematicMeshData } from '@/lab/schematic-preview/mesh-worker'
import type { SchematicPreviewScene } from '@/lab/schematic-preview/scene'
import {
    cancelUndergroundVolume,
    loadUndergroundVolume,
    nextUndergroundRegion,
    type UndergroundLayer,
    type UndergroundMesh,
    undergroundOrigin,
    type UndergroundQuery,
} from '@/lab/seed-map/underground'

const props = defineProps<{ query: UndergroundQuery }>()
const emit = defineEmits<{ close: [] }>()
const { formatMessage } = useVIntl()
const messages = defineMessages({
    title: {
        id: 'app.lab.seed-map.underground-3d.title',
        defaultMessage: '3D underground · X-Ray',
    },
    back: { id: 'app.lab.seed-map.underground-3d.back', defaultMessage: 'Back to map' },
    generate: {
        id: 'app.lab.seed-map.underground-3d.generate',
        defaultMessage: 'Generate around camera',
    },
    cancel: { id: 'app.lab.seed-map.underground-3d.cancel', defaultMessage: 'Cancel generation' },
    generating: {
        id: 'app.lab.seed-map.underground-3d.generating',
        defaultMessage: 'Computing terrain, caves and minerals…',
    },
    meshing: {
        id: 'app.lab.seed-map.underground-3d.meshing',
        defaultMessage: 'Building textured cave boundaries…',
    },
    paused: {
        id: 'app.lab.seed-map.underground-3d.paused',
        defaultMessage: 'Click the view to explore',
    },
    controls: {
        id: 'app.lab.seed-map.underground-3d.controls',
        defaultMessage:
            'Mouse: look · WASD: move · Space / Shift: up / down · Wheel: speed · Esc: release mouse. Free flight, without collision.',
    },
    terrain: { id: 'app.lab.seed-map.underground-3d.terrain', defaultMessage: 'Cave boundaries' },
    water: { id: 'app.lab.seed-map.underground-3d.water', defaultMessage: 'Water (estimated)' },
    lava: { id: 'app.lab.seed-map.underground-3d.lava', defaultMessage: 'Lava (estimated)' },
    ores: { id: 'app.lab.seed-map.underground-3d.ores', defaultMessage: 'Mineral candidates' },
    limits: {
        id: 'app.lab.seed-map.underground-3d.limits',
        defaultMessage:
            'Only cavity-facing rock is drawn; other solid rock is hidden. Stone / deepslate / netherrack are representative wall materials. Water is estimated only above the terrain surface below Y=63; underground aquifers are omitted. Lava fills empty space below Y={lava}. Fluids use static original textures. Mineral replacement, air-exposure rules, decorations and player changes are not fully simulated.',
    },
    range: {
        id: 'app.lab.seed-map.underground-3d.range',
        defaultMessage:
            'Loaded 64 × 64 × 64 blocks: X {x}…{maxX}, Y {y}…{maxY}, Z {z}…{maxZ}. Nearby regions load automatically as you move.',
    },
    autoPaused: {
        id: 'app.lab.seed-map.underground-3d.auto-paused',
        defaultMessage: 'Automatic loading is paused. Generate around the camera to resume.',
    },
    position: {
        id: 'app.lab.seed-map.underground-3d.position',
        defaultMessage: 'X {x} · Y {y} · Z {z} · {speed} blocks/s',
    },
    failed: {
        id: 'app.lab.seed-map.underground-3d.failed',
        defaultMessage:
            'Could not generate this region. Your previous view is kept. Retry or return to the map.',
    },
    retry: { id: 'app.lab.seed-map.underground-3d.retry', defaultMessage: 'Retry region' },
    truncated: {
        id: 'app.lab.seed-map.underground-3d.truncated',
        defaultMessage: 'The mineral candidate limit was reached; some deposits are omitted.',
    },
    contextLost: {
        id: 'app.lab.seed-map.underground-3d.context-lost',
        defaultMessage: 'The 3D graphics context was lost. Return to the map and reopen this view.',
    },
    noAir: {
        id: 'app.lab.seed-map.underground-3d.no-air',
        defaultMessage:
            'No open cave was found near this position. You can fly through rock or generate another region.',
    },
})
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')
const busy = ref(true)
const autoPaused = ref(false)
const stage = ref<'generating' | 'meshing'>('generating')
const failed = ref(false)
const locked = ref(false)
const contextLost = ref(false)
const ready = ref(false)
const truncated = ref(false)
const noAir = ref(false)
const speed = ref(8)
const layers = reactive<Record<UndergroundLayer, boolean>>({
    terrain: true,
    water: true,
    lava: true,
    ores: true,
})
const origin = ref<[number, number, number]>(undergroundOrigin(props.query))
const position = ref<[number, number, number]>([props.query.x, props.query.y, props.query.z])
let scene: SchematicPreviewScene | undefined
let texture: CanvasTexture | undefined
let textureUvs: Record<string, [number, number, number, number]>
let worker: Worker | undefined
let rejectWorker: ((error: Error) => void) | undefined
let task = 0
let timer: ReturnType<typeof setInterval> | undefined
let lastQuery = props.query
let disposed = false
let initialCanceled = false
let initialization: Promise<void> | undefined
const resourceAbort = new AbortController()
const lavaLevel = computed(() =>
    props.query.dimension === 'nether'
        ? 31
        : Number(props.query.version.split('.')[0]) >= 26 ||
            /^1\.(?:1[89]|2\d)(?:\.|$)/.test(props.query.version)
          ? -54
          : 10,
)
const range = computed(() =>
    formatMessage(messages.range, {
        x: origin.value[0],
        y: origin.value[1],
        z: origin.value[2],
        maxX: origin.value[0] + 63,
        maxY: origin.value[1] + 63,
        maxZ: origin.value[2] + 63,
    }),
)
const positionText = computed(() =>
    formatMessage(messages.position, {
        x: Math.floor(position.value[0]),
        y: Math.floor(position.value[1]),
        z: Math.floor(position.value[2]),
        speed: speed.value,
    }),
)

function cancel() {
    autoPaused.value = true
    if (!scene) initialCanceled = true
    const previous = task
    task = 0
    if (previous) void cancelUndergroundVolume(previous).catch(() => undefined)
    worker?.terminate()
    worker = undefined
    rejectWorker?.(new Error('Canceled'))
    rejectWorker = undefined
    busy.value = false
}

function meshVolume(volume: Awaited<ReturnType<typeof loadUndergroundVolume>>) {
    return new Promise<UndergroundMesh>((resolve, reject) => {
        worker = new Worker(
            new URL('../../../lab/seed-map/underground-worker.ts', import.meta.url),
            { type: 'module' },
        )
        rejectWorker = reject
        worker.onmessage = (event: MessageEvent<{ mesh?: UndergroundMesh; error?: string }>) => {
            worker?.terminate()
            worker = undefined
            rejectWorker = undefined
            if (event.data.mesh) resolve(event.data.mesh)
            else reject(new Error(event.data.error ?? 'Underground mesh failed'))
        }
        worker.onerror = () => {
            worker?.terminate()
            worker = undefined
            rejectWorker = undefined
            reject(new Error('Underground mesh worker failed'))
        }
        worker.postMessage({ ...volume, textureUvs }, [volume.blocks.buffer])
    })
}

async function generate(query: UndergroundQuery = lastQuery) {
    cancel()
    autoPaused.value = false
    lastQuery = { ...query }
    const id = Date.now() * 1000 + Math.floor(Math.random() * 1000)
    task = id
    busy.value = true
    failed.value = false
    stage.value = 'generating'
    const nextOrigin = undergroundOrigin(query)
    try {
        const volume = await loadUndergroundVolume(
            { ...query, x: nextOrigin[0], y: nextOrigin[1], z: nextOrigin[2] },
            id,
        )
        if (disposed || task !== id) return
        stage.value = 'meshing'
        const mesh = await meshVolume(volume)
        if (disposed || task !== id || !scene) return
        const camera = scene.getCameraPosition()
        const nextPosition: [number, number, number] = ready.value
            ? (camera.map((value, axis) => value + origin.value[axis] - nextOrigin[axis]) as [
                  number,
                  number,
                  number,
              ])
            : mesh.spawn
        origin.value = nextOrigin
        truncated.value = volume.truncated
        noAir.value = mesh.noAir
        scene.setRegions(
            (Object.keys(layers) as UndergroundLayer[]).map((id) => ({
                id,
                min: [0, 0, 0],
                max: [63, 63, 63],
            })),
        )
        const empty: SchematicMeshData = {
            positions: new Float32Array(),
            normals: new Float32Array(),
            uvs: new Float32Array(),
            colors: new Float32Array(),
            blockPositions: new Float32Array(),
        }
        for (const layer of Object.keys(layers) as UndergroundLayer[]) {
            const fluid = layer === 'water'
            scene.setChunk(
                layer,
                [0, 0, 0],
                fluid ? empty : mesh.layers[layer],
                fluid ? mesh.layers[layer] : empty,
            )
            scene.setRegionVisible(layer, layers[layer])
        }
        scene.setCameraPosition(nextPosition)
        scene.setViewMode('walk')
        ready.value = true
        updatePosition()
    } catch (error) {
        if (!disposed && task === id) {
            console.error('Underground preview failed', error)
            failed.value = true
        }
    } finally {
        if (task === id) {
            busy.value = false
            task = 0
        }
    }
}

function updatePosition() {
    if (scene && ready.value)
        position.value = scene
            .getCameraPosition()
            .map((value, axis) => value + origin.value[axis]) as [number, number, number]
    if (
        !ready.value ||
        !locked.value ||
        busy.value ||
        autoPaused.value ||
        failed.value ||
        contextLost.value ||
        disposed
    )
        return
    const next = nextUndergroundRegion(props.query, origin.value, position.value)
    if (next) void generate(next)
}
function generateAtCamera() {
    updatePosition()
    void generate({
        ...props.query,
        x: position.value[0],
        y: position.value[1],
        z: position.value[2],
    })
}
watch(layers, () => {
    for (const layer of Object.keys(layers) as UndergroundLayer[])
        scene?.setRegionVisible(layer, layers[layer])
})

function initialize() {
    busy.value = true
    failed.value = false
    initialization ??= initializeScene().finally(() => {
        initialization = undefined
    })
    return initialization
}

async function initializeScene() {
    try {
        const [{ SchematicPreviewScene }, builtin, three, { configureSchematicTexture }] =
            await Promise.all([
                import('@/lab/schematic-preview/scene'),
                import('@/lab/schematic-preview/builtin-resources'),
                import('three'),
                import('@/lab/schematic-preview/resources'),
            ])
        if (disposed || !canvas.value) return
        const image = await builtin.loadBuiltinAtlas(resourceAbort.signal)
        const atlas = document.createElement('canvas')
        atlas.width = builtin.BUILTIN_ATLAS_WIDTH
        atlas.height = builtin.BUILTIN_ATLAS_HEIGHT
        const context = atlas.getContext('2d')
        if (!context || disposed) {
            image.close()
            if (disposed) return
            throw new Error('Unable to create the Minecraft texture atlas')
        }
        context.drawImage(image, 0, 0)
        image.close()
        texture = configureSchematicTexture(new three.CanvasTexture(atlas))
        texture.colorSpace = three.SRGBColorSpace
        textureUvs = builtin.builtinTextureUvs()
        scene = new SchematicPreviewScene({
            canvas: canvas.value,
            onSelect: () => undefined,
            onFocus: () => undefined,
            onViewModeChange: () => undefined,
            onContextLost: () => {
                contextLost.value = true
            },
            onContextRestored: () => {
                contextLost.value = false
            },
            onWalkLockChange: (value) => {
                locked.value = value
            },
            onWalkSpeedChange: (value) => {
                speed.value = value
            },
            requireWalkLock: true,
            doubleSided: true,
            ...(isTauri() && platform() === 'macos'
                ? {
                      onNativeWalkLock: async () => {
                          const window = getCurrentWindow()
                          try {
                              await window.setCursorGrab(true)
                              await window.setCursorVisible(false)
                              return true
                          } catch {
                              await window.setCursorGrab(false).catch(() => undefined)
                              return false
                          }
                      },
                      onNativeWalkUnlock: async () => {
                          const window = getCurrentWindow()
                          try {
                              await window.setCursorVisible(true)
                          } finally {
                              await window.setCursorGrab(false)
                          }
                      },
                  }
                : {}),
        })
        scene.setTexture(texture)
        scene.setGridVisible(false)
        scene.setBoundsVisible(false)
        timer = setInterval(updatePosition, 150)
        if (!initialCanceled) await generate(props.query)
        else busy.value = false
    } catch (error) {
        if (!disposed) {
            busy.value = false
            failed.value = true
            console.error('Underground renderer failed', error)
        }
    }
}
function retry() {
    autoPaused.value = false
    if (scene) void generate()
    else {
        initialCanceled = false
        void initialize()
    }
}
onMounted(initialize)
onBeforeUnmount(() => {
    disposed = true
    cancel()
    resourceAbort.abort()
    if (timer) clearInterval(timer)
    scene?.dispose()
    texture?.dispose()
})
</script>

<template>
    <div
        class="absolute inset-0 z-30 flex min-h-0 flex-col bg-surface-1"
        data-onboarding-id="seed-map-underground-3d-view"
        @pointerdown.stop
        @wheel.stop
    >
        <div
            class="flex flex-wrap items-center gap-2 border-b border-surface-4 bg-surface-2 p-2"
            @keydown.stop
        >
            <Button size="2xs" type="quiet" @click="emit('close')"
                ><ArrowLeftIcon />{{ formatMessage(messages.back) }}</Button
            >
            <strong class="mr-auto text-sm">{{ formatMessage(messages.title) }}</strong>
            <Button v-if="busy" size="2xs" type="outlined" @click="cancel">{{
                formatMessage(messages.cancel)
            }}</Button>
            <Button
                v-else
                size="2xs"
                type="outlined"
                :disabled="contextLost"
                @click="failed || !scene ? retry() : generateAtCamera()"
                ><RefreshCwIcon />{{
                    formatMessage(failed ? messages.retry : messages.generate)
                }}</Button
            >
        </div>
        <div class="relative min-h-0 flex-1 overflow-hidden">
            <canvas
                ref="canvas"
                class="absolute inset-0 block size-full"
                :aria-label="formatMessage(messages.title)"
            />
            <div
                v-if="!locked"
                class="pointer-events-none absolute inset-x-3 top-3 flex flex-col gap-2"
            >
                <p v-if="ready" class="m-0 self-center rounded-lg bg-surface-2 px-3 py-2 text-sm">
                    {{ formatMessage(messages.paused) }}
                </p>
                <div
                    class="pointer-events-auto flex max-h-[min(26rem,60%)] w-[min(26rem,100%)] flex-col gap-2 overflow-y-auto rounded-lg border border-surface-4 bg-surface-2 p-3 text-xs"
                    @keydown.stop
                >
                    <p v-if="busy" role="status" class="m-0">
                        {{ formatMessage(messages[stage]) }}
                    </p>
                    <p v-if="autoPaused" role="status" class="m-0">
                        {{ formatMessage(messages.autoPaused) }}
                    </p>
                    <p v-if="failed" role="alert" class="m-0">
                        {{ formatMessage(messages.failed) }}
                    </p>
                    <p v-if="contextLost" role="alert" class="m-0">
                        {{ formatMessage(messages.contextLost) }}
                    </p>
                    <div class="flex flex-wrap gap-x-4 gap-y-2">
                        <label
                            v-for="layer in ['terrain', 'water', 'lava', 'ores'] as const"
                            :key="layer"
                            class="flex items-center gap-2"
                            ><Toggle v-model="layers[layer]" small />{{
                                formatMessage(messages[layer])
                            }}</label
                        >
                    </div>
                    <p class="m-0">{{ formatMessage(messages.controls) }}</p>
                    <p class="m-0 text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.limits, { lava: lavaLevel }) }}
                    </p>
                    <p v-if="ready" class="m-0 text-[var(--color-text-tertiary)]">{{ range }}</p>
                    <p v-if="truncated" class="m-0">{{ formatMessage(messages.truncated) }}</p>
                    <p v-if="noAir" class="m-0">{{ formatMessage(messages.noAir) }}</p>
                </div>
            </div>
            <div
                v-if="locked"
                class="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 text-white"
                aria-hidden="true"
            >
                +
            </div>
            <p
                class="pointer-events-none absolute bottom-2 left-2 m-0 rounded-lg bg-surface-2 px-2 py-1 text-xs tabular-nums"
            >
                {{ positionText }}
            </p>
        </div>
    </div>
</template>
