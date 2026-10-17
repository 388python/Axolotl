import '@/test/visual-styles.scss'

import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import type { SeedMapTile, SeedMapTileRequest } from '@/lab/seed-map/backend'
import type { SeedMapWorkspace } from '@/lab/seed-map/workspace'

import LabSeedMap from './LabSeedMap.vue'

const backend = vi.hoisted(() => ({
    tiles: [] as { request: SeedMapTileRequest; resolve: (tile: SeedMapTile) => void }[],
    spawns: [] as {
        seed: string
        resolve: (spawn: { x: number; z: number; approximate: boolean }) => void
    }[],
    features: vi.fn(async () => []),
    ores: vi.fn(async () => []),
}))

vi.mock('vue-router', () => ({ useRoute: () => ({ query: {} }) }))
vi.mock('@/lab/recipe-generator/item-images', () => ({
    loadMinecraftItemImage: () => Promise.resolve({ src: '' }),
}))
vi.mock('@/components/lab/seed-map/SeedMapBiomePicker.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/components/lab/seed-map/SeedMapCopyrightModal.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/components/lab/seed-map/SeedMapStructureDetails.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/components/lab/seed-map/SeedMapWorldImportModal.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/lab/seed-map/backend.ts', async (importOriginal) => ({
    ...(await importOriginal<typeof import('@/lab/seed-map/backend')>()),
    getSeedMapProfiles: async () => [
        {
            edition: 'java',
            version: '26.3',
            available: true,
            dimensions: ['overworld', 'nether', 'end'],
            ores: true,
        },
    ],
    renderSeedMapTile: (request: SeedMapTileRequest) =>
        new Promise((resolve) => backend.tiles.push({ request, resolve })),
    getSeedMapSpawn: (seed: string) =>
        new Promise((resolve) => backend.spawns.push({ seed, resolve })),
    findSeedMapFeatures: backend.features,
    scanSeedMapOres: backend.ores,
}))
vi.mock('@modrinth/ui', async () => {
    const { defineComponent, h, ref } = await import('vue')
    const control = defineComponent({
        setup:
            (_, { slots }) =>
            () =>
                h('div', slots.default?.()),
    })
    return {
        Admonition: control,
        Checkbox: control,
        Combobox: control,
        PopoutMenu: control,
        Slider: control,
        Toggle: control,
        Button: (await import('../../../../packages/ui/src/components/base/buttons/Button.vue'))
            .default,
        StyledInput: defineComponent({
            props: ['modelValue'],
            emits: ['update:modelValue'],
            setup:
                (props, { emit }) =>
                () =>
                    h('input', {
                        value: props.modelValue,
                        onInput: (event: Event) =>
                            emit('update:modelValue', (event.target as HTMLInputElement).value),
                    }),
        }),
        defineMessages: (messages: unknown) => messages,
        injectNotificationManager: () => ({ handleError: vi.fn(), addNotification: vi.fn() }),
        useRelativeTime: () => () => '',
        useVIntl: () => ({
            locale: ref('en-US'),
            formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
        }),
    }
})

type MapVm = {
    workspace: SeedMapWorkspace
    resizeCanvas: () => void
    scheduleRefresh: (delay?: number) => void
    drawMap: () => void
}
let wrapper: VueWrapper | undefined
let closed: ReturnType<typeof vi.spyOn>[] = []

beforeEach(() => {
    localStorage.removeItem('axolotl.lab.seed-map.v1')
    localStorage.removeItem('axolotl.lab.seed-map.history.v1')
    backend.tiles = []
    backend.spawns = []
    backend.features.mockClear()
    backend.ores.mockClear()
})

afterEach(async () => {
    wrapper?.unmount()
    wrapper = undefined
    for (const job of backend.tiles.splice(0)) await complete(job, '#00ff00')
    for (const spy of closed) spy.mockRestore()
    closed = []
})

async function openMap() {
    wrapper = mount(LabSeedMap, {
        attachTo: document.body,
        global: {
            directives: { tooltip: () => {} },
            provide: { replayOnboarding: async () => {} },
        },
    })
    await flushPromises()
    const canvas = wrapper.get('canvas').element as HTMLCanvasElement
    canvas.style.width = '512px'
    canvas.style.height = '256px'
    const vm = wrapper.vm as unknown as MapVm
    vm.resizeCanvas()
    vm.scheduleRefresh(0)
    await expect.poll(() => backend.tiles.length).toBeGreaterThan(0)
    return { vm, canvas }
}

async function complete(job: (typeof backend.tiles)[number], color: string) {
    const source = new OffscreenCanvas(job.request.width, job.request.height)
    const context = source.getContext('2d')!
    context.fillStyle = color
    context.fillRect(0, 0, source.width, source.height)
    const bitmap = source.transferToImageBitmap()
    closed.push(vi.spyOn(bitmap, 'close'))
    job.resolve({
        epoch: job.request.epoch,
        width: source.width,
        height: source.height,
        bitmap,
        approximate: true,
    })
    await flushPromises()
}

it('renders seed changes while spawn is pending and deduplicates spawn work across map refreshes', async () => {
    const { vm } = await openMap()
    expect(backend.spawns).toHaveLength(1)
    expect(backend.features).not.toHaveBeenCalled()
    vm.workspace.seed = '456'
    await flushPromises()
    vm.scheduleRefresh(0)
    await expect.poll(() => backend.spawns.length).toBe(2)
    vm.workspace.center.x += 1
    vm.scheduleRefresh(0)
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(backend.spawns).toHaveLength(2)
    for (const job of backend.tiles.splice(0)) await complete(job, '#ff0000')
    await expect.poll(() => backend.tiles.some((job) => job.request.seed === '456')).toBe(true)
    backend.spawns[0].resolve({ x: 999, z: 999, approximate: false })
    await flushPromises()
    expect(vm.workspace.center.x).not.toBe(999)
    backend.spawns[1].resolve({ x: 128, z: 64, approximate: false })
    await flushPromises()
    expect(vm.workspace.center).toEqual({ x: 128, z: 64 })
})

it('keeps real canvas pixels across LoD changes and progressively paints new detail over old tiles', async () => {
    const { vm, canvas } = await openMap()
    const initial = backend.tiles.splice(0)
    for (const job of initial) await complete(job, '#ff0000')
    for (let batch = 0; batch < 4 && !backend.features.mock.calls.length; batch++) {
        await new Promise((resolve) => setTimeout(resolve, 30))
        for (const job of backend.tiles.splice(0)) await complete(job, '#ff0000')
    }
    vm.drawMap()
    const sample = () => [
        ...canvas
            .getContext('2d')!
            .getImageData(
                Math.round(300 * devicePixelRatio),
                Math.round(150 * devicePixelRatio),
                1,
                1,
            ).data,
    ]
    expect(sample()).toEqual([255, 0, 0, 255])
    expect(backend.features).toHaveBeenCalledTimes(1)
    vm.workspace.zoom = 0
    await flushPromises()
    vm.scheduleRefresh(0)
    vm.drawMap()
    expect(sample()).toEqual([255, 0, 0, 255])
    expect(closed.every((spy) => spy.mock.calls.length === 0)).toBe(true)
    for (
        let batch = 0;
        batch < 8 &&
        !backend.tiles.some((job) => job.request.scale === 1 && job.request.width === 256);
        batch++
    ) {
        for (const job of backend.tiles.splice(0)) await complete(job, '#ff0000')
        await new Promise((resolve) => setTimeout(resolve, 20))
    }
    const fine = backend.tiles.find(
        (job) =>
            job.request.scale === 1 &&
            job.request.width === 256 &&
            job.request.x === 0 &&
            job.request.z === 0,
    )
    expect(fine).toBeDefined()
    backend.tiles.splice(backend.tiles.indexOf(fine!), 1)
    await complete(fine!, '#00ff00')
    vm.drawMap()
    expect(sample()).toEqual([0, 255, 0, 255])
    expect(closed.every((spy) => spy.mock.calls.length === 0)).toBe(true)
})

it('defers ore scanning until base coverage and invalidates pending work on a seed change', async () => {
    const { vm } = await openMap()
    vm.workspace.displayMode = 'ores'
    vm.workspace.zoom = -1
    await flushPromises()
    vm.scheduleRefresh(0)
    await new Promise((resolve) => setTimeout(resolve, 220))
    expect(backend.ores).not.toHaveBeenCalled()
    for (let batch = 0; batch < 10 && !backend.ores.mock.calls.length; batch++) {
        for (const job of backend.tiles.splice(0)) await complete(job, '#ff0000')
        await new Promise((resolve) => setTimeout(resolve, 60))
    }
    await expect.poll(() => backend.ores.mock.calls.length).toBeGreaterThan(0)
    backend.ores.mockClear()
    vm.workspace.seed = '789'
    await flushPromises()
    vm.scheduleRefresh(0)
    await new Promise((resolve) => setTimeout(resolve, 220))
    expect(backend.ores).not.toHaveBeenCalled()
})

it.each([
    { mode: 'structures', dimension: 'overworld' },
    { mode: 'structures', dimension: 'nether' },
    { mode: 'structures', dimension: 'end' },
    { mode: 'ores', dimension: 'overworld' },
    { mode: 'ores', dimension: 'nether' },
] as const)(
    'toggles every $mode layer button in the $dimension, including its visible selected state',
    async ({ mode, dimension }) => {
        const { vm } = await openMap()
        vm.workspace.dimension = dimension
        vm.workspace.displayMode = mode
        await flushPromises()
        const buttons = wrapper!.findAll('.layer-options > button')
        expect(buttons.length).toBeGreaterThan(0)
        for (const button of buttons) {
            const wasSelected = button.attributes('aria-pressed') === 'true'
            await userEvent.click(button.element)
            await flushPromises()
            expect(button.attributes('aria-pressed')).toBe(String(!wasSelected))
            expect(button.classes()).toContain(
                wasSelected ? 'button-frame--base' : 'button-frame--colored',
            )
            await userEvent.click(button.element)
            await flushPromises()
            expect(button.attributes('aria-pressed')).toBe(String(wasSelected))
            expect(button.classes()).toContain(
                wasSelected ? 'button-frame--colored' : 'button-frame--base',
            )
        }
    },
)

it('loads the provided camp and fossil PNGs and omits the removed end-island layer', async () => {
    const { vm } = await openMap()
    const camp = wrapper!.get('.layer-options img[src$="/abandoned_camp.png"]')
        .element as HTMLImageElement
    await camp.decode()
    expect(camp.naturalWidth).toBe(26)
    vm.workspace.dimension = 'nether'
    await flushPromises()
    const fossil = wrapper!.get('.layer-options img[src$="/nether_fossil.png"]')
        .element as HTMLImageElement
    await fossil.decode()
    expect(fossil.naturalWidth).toBe(26)
    vm.workspace.dimension = 'end'
    await flushPromises()
    expect(wrapper!.findAll('.layer-options > button')).toHaveLength(3)
})
