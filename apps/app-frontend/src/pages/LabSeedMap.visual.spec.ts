import '@/test/visual-styles.scss'

import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'
import { defineComponent, h, Suspense, Transition, type VNode } from 'vue'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'

import type { SeedMapStructureDetails, SeedMapStructurePiece } from '@/lab/seed-map/advanced'
import type { SeedMapTile, SeedMapTileRequest } from '@/lab/seed-map/backend'
import type { SeedMapFeature } from '@/lab/seed-map/features'
import { createDefaultSeedMapWorkspace, type SeedMapWorkspace } from '@/lab/seed-map/workspace'

import LabSeedMap from './LabSeedMap.vue'

const backend = vi.hoisted(() => ({
    tiles: [] as { request: SeedMapTileRequest; resolve: (tile: SeedMapTile) => void }[],
    spawns: [] as {
        seed: string
        resolve: (spawn: { x: number; z: number; approximate: boolean }) => void
    }[],
    features: vi.fn(async () => []),
    ores: vi.fn(async () => []),
    details: vi.fn(),
}))

vi.mock('@/lab/seed-map/advanced', () => ({
    getSeedMapStructureDetails: backend.details,
    getSeedMapContainerLoot: vi.fn(),
}))

vi.mock('vue-router', async (importOriginal) => ({
    ...(await importOriginal<typeof import('vue-router')>()),
    useRoute: () => ({ query: {} }),
}))
vi.mock('@/lab/recipe-generator/item-images', () => ({
    loadMinecraftItemImage: () => Promise.resolve({ src: '' }),
}))
vi.mock('@/components/lab/seed-map/SeedMapBiomePicker.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/components/lab/seed-map/SeedMapCopyrightModal.vue', () => ({
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
    features: SeedMapFeature[]
    selection: { x: number; z: number; feature?: SeedMapFeature } | null
    focusedPiece: SeedMapStructurePiece | null
    selectAt: (x: number, y: number) => void
    worldToScreen: (x: number, z: number) => { x: number; y: number }
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
    backend.details.mockReset().mockResolvedValue({
        supported: false,
        truncated: false,
        pieces: [],
        containers: [],
        limits: ['layout_unavailable'],
    })
})

it.each([-2, 0, 0.5])(
    'opens a saved workspace at zoom %s without reading uninitialized layout dependencies',
    async (zoom) => {
        localStorage.setItem(
            'axolotl.lab.seed-map.v1',
            JSON.stringify({ ...createDefaultSeedMapWorkspace(), zoom }),
        )
        const { vm } = await openMap()
        expect(vm.workspace.zoom).toBe(zoom)
        expect(wrapper!.get('canvas').exists()).toBe(true)
    },
)

it('can leave a close-zoom map with pending work and navigate back through the app transition shell', async () => {
    localStorage.setItem(
        'axolotl.lab.seed-map.v1',
        JSON.stringify({ ...createDefaultSeedMapWorkspace(), zoom: 0 }),
    )
    const errors: unknown[] = []
    const router = createRouter({
        history: createMemoryHistory(),
        routes: [
            { path: '/lab/seed-map', component: LabSeedMap },
            {
                path: '/other',
                component: defineComponent({
                    render: () => h('div', { 'data-testid': 'other-page' }, 'Other page'),
                }),
            },
        ],
    })
    router.onError((error) => errors.push(error))
    await router.push('/lab/seed-map')
    await router.isReady()
    const shell = defineComponent({
        render: () =>
            h(
                RouterView,
                {},
                {
                    default: ({
                        Component,
                        route,
                    }: {
                        Component: VNode
                        route: { path: string }
                    }) =>
                        h(
                            Transition,
                            { css: false, appear: true },
                            {
                                default: () =>
                                    h('div', { key: route.path }, [
                                        h(Suspense, {}, { default: () => Component }),
                                    ]),
                            },
                        ),
                },
            ),
    })
    wrapper = mount(shell, {
        attachTo: document.body,
        global: {
            plugins: [router],
            directives: { tooltip: () => {} },
            provide: { replayOnboarding: async () => {} },
        },
    })
    await vi.waitFor(() => expect(wrapper!.find('canvas').exists()).toBe(true))
    await vi.waitFor(() => expect(backend.tiles.length).toBeGreaterThan(0))
    await router.push('/other')
    await flushPromises()
    expect(wrapper.get('[data-testid="other-page"]').text()).toBe('Other page')
    for (const job of backend.tiles.splice(0)) await complete(job, '#00ff00')
    await router.push('/lab/seed-map')
    await flushPromises()
    expect(wrapper.find('canvas').exists()).toBe(true)
    await router.push('/other')
    await flushPromises()
    expect(wrapper.find('[data-testid="other-page"]').exists()).toBe(true)
    expect(errors).toEqual([])
})

it('automatically paints visible layouts without a selection, shares requests with details and resizes the split', async () => {
    await page.viewport(1280, 1100)
    document.documentElement.classList.add('dark-mode')
    const result: SeedMapStructureDetails = {
        supported: true,
        truncated: false,
        containers: [],
        limits: ['relative_height'],
        pieces: [
            {
                id: 0,
                name: 'SHPR',
                rotation: 0,
                positionKnown: true,
                position: { x: -40, y: 64, z: -20 },
                min: { x: -40, y: 64, z: -20 },
                max: { x: 40, y: 75, z: 20 },
            },
        ],
    }
    backend.details.mockResolvedValue(result)
    const { vm, canvas } = await openMap()
    ;(wrapper!.element as HTMLElement).style.cssText =
        'width: 1200px; height: 1000px; background: var(--surface-1); color: var(--color-text-primary)'
    canvas.style.width = ''
    canvas.style.height = ''
    vm.resizeCanvas()
    vm.workspace.zoom = 0
    vm.workspace.showGrid = false
    vm.workspace.showSpawn = false
    vm.workspace.visibleFeatures = ['stronghold']
    vm.features = [{ kind: 'stronghold', x: 0, z: 0, approximate: true }]
    await flushPromises()
    await vi.waitFor(() => expect(backend.details).toHaveBeenCalledOnce())
    expect(vm.selection).toBeNull()
    vm.drawMap()
    const sample = () => {
        const ratio = window.devicePixelRatio || 1
        return [
            ...canvas
                .getContext('2d')!
                .getImageData(
                    Math.floor((canvas.clientWidth / 2 + 25) * ratio),
                    Math.floor((canvas.clientHeight / 2) * ratio),
                    1,
                    1,
                ).data,
        ]
    }
    const overlay = sample()
    vm.workspace.zoom = 1
    await flushPromises()
    vm.drawMap()
    expect(sample()).not.toEqual(overlay)
    vm.workspace.zoom = 0
    await flushPromises()
    vm.selectAt(canvas.clientWidth / 2 + 25, canvas.clientHeight / 2)
    await flushPromises()
    await vi.waitFor(() => expect(wrapper!.find('aside').exists()).toBe(true))
    expect(backend.details).toHaveBeenCalledOnce()
    expect(vm.focusedPiece?.id).toBe(0)
    const divider = wrapper!.get('[role="separator"]')
    const mapRegion = wrapper!.get('.seed-map-canvas-shell').element as HTMLElement
    const initialWidth = mapRegion.clientWidth
    await divider.trigger('keydown', { key: 'ArrowLeft' })
    await vi.waitFor(() => expect(mapRegion.clientWidth).toBeLessThan(initialWidth))
    const keyboardWidth = mapRegion.clientWidth
    await userEvent.dragAndDrop(divider.element, mapRegion)
    await vi.waitFor(() => expect(mapRegion.clientWidth).toBeLessThan(keyboardWidth))
    await divider.trigger('dblclick')
    await vi.waitFor(() => expect(Number(divider.attributes('aria-valuenow'))).toBe(62))
    await wrapper!.findAll('[role="tab"]')[1].trigger('click')
    await vi.waitFor(() =>
        expect(wrapper!.get('aside canvas').attributes('data-ready')).toBe('true'),
    )
    if (import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE)
        await page.screenshot({
            element: wrapper!.element,
            path: `${import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE}/seed-map-split-desktop.png`,
        })
    ;(wrapper!.element as HTMLElement).style.width = '650px'
    await vi.waitFor(() => expect(wrapper!.find('[role="separator"]').exists()).toBe(false))
    expect(wrapper!.element.scrollWidth).toBeLessThanOrEqual(wrapper!.element.clientWidth + 1)
    expect(mapRegion.getBoundingClientRect().bottom).toBeLessThanOrEqual(
        wrapper!.get('aside').element.getBoundingClientRect().top + 1,
    )
    if (import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE)
        await page.screenshot({
            element: wrapper!.element,
            path: `${import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE}/seed-map-split-narrow.png`,
        })
    await wrapper!.get('aside button[aria-label="Close structure details"]').trigger('click')
    expect(vm.selection).toBeNull()
    expect(wrapper!.find('aside').exists()).toBe(false)
})

it('keeps screen coordinates and painted canvas intact when opening, dragging and closing details', async () => {
    await page.viewport(1280, 1100)
    const { vm, canvas } = await openMap()
    ;(wrapper!.element as HTMLElement).style.cssText = 'width: 1200px; height: 1000px'
    canvas.style.width = ''
    canvas.style.height = ''
    await vi.waitFor(() => expect(canvas.clientWidth).toBeGreaterThan(1000))
    vm.resizeCanvas()
    vm.workspace.showSpawn = false
    vm.workspace.visibleFeatures = []
    await flushPromises()
    vm.drawMap()
    const center = { ...vm.workspace.center }
    const screenPoint = vm.worldToScreen(center.x - 200, center.z - 100)
    const dimensions = { width: canvas.width, height: canvas.height }
    const origin = canvas.getBoundingClientRect()
    const resets: MutationRecord[] = []
    const observer = new MutationObserver((records) => resets.push(...records))
    observer.observe(canvas, { attributes: true, attributeFilter: ['width', 'height'] })
    const pixel = () => [...canvas.getContext('2d')!.getImageData(40, 40, 1, 1).data]
    const painted = pixel()
    const frames: number[][] = []
    let recording = true
    const sample = () => {
        if (recording) {
            frames.push(pixel())
            requestAnimationFrame(sample)
        }
    }
    requestAnimationFrame(sample)
    try {
        vm.selection = {
            x: 0,
            z: 0,
            feature: { kind: 'stronghold', x: 0, z: 0, approximate: true },
        }
        await vi.waitFor(() => expect(wrapper!.find('[role="separator"]').exists()).toBe(true))
        const divider = wrapper!.get('[role="separator"]')
        for (const key of ['ArrowLeft', 'ArrowLeft', 'ArrowRight', 'End', 'Home']) {
            await divider.trigger('keydown', { key })
            await new Promise((resolve) => requestAnimationFrame(resolve))
            expect(vm.worldToScreen(center.x - 200, center.z - 100)).toEqual(screenPoint)
            expect(canvas.width).toBe(dimensions.width)
            expect(canvas.height).toBe(dimensions.height)
        }
        await userEvent.dragAndDrop(divider.element, wrapper!.get('.seed-map-canvas-shell').element)
        await wrapper!.get('aside button[aria-label="Close structure details"]').trigger('click')
        await new Promise((resolve) => requestAnimationFrame(resolve))
        expect(vm.workspace.center).toEqual(center)
        expect(vm.worldToScreen(center.x - 200, center.z - 100)).toEqual(screenPoint)
        expect(canvas.getBoundingClientRect().left).toBe(origin.left)
        expect(canvas.getBoundingClientRect().top).toBe(origin.top)
        expect(canvas.width).toBe(dimensions.width)
        expect(canvas.height).toBe(dimensions.height)
        expect(resets).toHaveLength(0)
        expect(frames.length).toBeGreaterThan(0)
        expect(
            frames.every((frame) => frame.every((value, index) => value === painted[index])),
        ).toBe(true)
    } finally {
        recording = false
        observer.disconnect()
    }
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
