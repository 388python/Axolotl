import '@/test/visual-styles.scss'

import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import { SchematicPreviewScene } from '@/lab/schematic-preview/scene'
import { buildUndergroundMesh } from '@/lab/seed-map/underground-mesh'

import SeedMapUndergroundPreview from './SeedMapUndergroundPreview.vue'

const backend = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: backend.invoke, isTauri: () => false }))
vi.mock('@modrinth/ui', async () => {
    const { defineComponent, h } = await import('vue')
    return {
        defineMessages: (messages: unknown) => messages,
        useVIntl: () => ({
            formatMessage: (
                message: { defaultMessage: string },
                values: Record<string, unknown> = {},
            ) =>
                message.defaultMessage.replace(/\{(\w+)\}/g, (_, key) =>
                    String(values[key] ?? key),
                ),
        }),
        Button: defineComponent({
            setup:
                (_, { slots }) =>
                () =>
                    h('button', slots.default?.()),
        }),
        Toggle: defineComponent({
            props: ['modelValue'],
            emits: ['update:modelValue'],
            setup:
                (props, { emit }) =>
                () =>
                    h('input', {
                        type: 'checkbox',
                        checked: props.modelValue,
                        onChange: (event: Event) =>
                            emit('update:modelValue', (event.target as HTMLInputElement).checked),
                    }),
        }),
    }
})

let wrapper: VueWrapper | undefined
let host: HTMLDivElement | undefined
afterEach(() => {
    wrapper?.unmount()
    host?.remove()
    wrapper = undefined
    vi.clearAllMocks()
})

it('targets visible block coordinates after chunk translation and ignores hidden or removed chunks', () => {
    host = document.createElement('div')
    host.style.cssText = 'position:relative;width:600px;height:500px'
    document.body.append(host)
    const canvas = document.createElement('canvas')
    host.append(canvas)
    const scene = new SchematicPreviewScene({
        canvas,
        onSelect: () => undefined,
        onFocus: () => undefined,
        onContextLost: () => undefined,
        onContextRestored: () => undefined,
        onViewModeChange: () => undefined,
        onWalkLockChange: () => undefined,
        onWalkSpeedChange: () => undefined,
        doubleSided: true,
    })
    try {
        const blocks = new Uint8Array(125)
        blocks[(2 * 5 + 2) * 5 + 2] = 1
        const mesh = buildUndergroundMesh({
            blocks,
            width: 5,
            height: 5,
            truncated: false,
            textureUvs: { 'minecraft:block/stone': [0, 0, 1, 1] },
        })
        scene.setRegions([{ id: 'terrain', min: [0, 0, 0], max: [5, 5, 5] }])
        scene.setChunk('terrain', [0, 0, 0], mesh.layers.terrain, mesh.layers.water)
        scene.setChunkOffset('terrain', [0, 0, 0], [10, 20, 30])
        scene.setCameraPosition([15, 25, 35])
        expect(scene.crosshairBlock()).toEqual([12, 22, 32])
        scene.setRegionVisible('terrain', false)
        expect(scene.crosshairBlock()).toBeUndefined()
        scene.setRegionVisible('terrain', true)
        scene.removeChunk('terrain', [0, 0, 0])
        expect(scene.crosshairBlock()).toBeUndefined()
    } finally {
        scene.dispose()
    }
})

it('loads textured cavity geometry, switches layers, keeps the view on failure and cancels pending generation on exit', async () => {
    const volume = () => {
        const result = new ArrayBuffer(16 + 18 * 18 * 384 + 18 * 18 * 2)
        const header = new DataView(result)
        ;[3, 0, 18, 384].forEach((value, index) => header.setUint32(index * 4, value, true))
        const blocks = new Uint8Array(result, 16, 18 * 18 * 384).fill(1)
        for (let x = 1; x < 17; x++)
            for (let z = 1; z < 17; z++)
                for (let y = 64; y < 112; y++) blocks[(x * 18 + z) * 384 + y] = 0
        for (let column = 0; column < 18 * 18; column++)
            header.setInt16(16 + blocks.length + column * 2, 100, true)
        blocks[(8 * 18 + 8) * 384 + 64] = 5
        blocks[(10 * 18 + 8) * 384 + 64] = 4
        blocks[(6 * 18 + 8) * 384 + 63] = 16
        return result
    }
    let fail = false
    let pending = false
    backend.invoke.mockImplementation((command: string) => {
        if (command.endsWith('seed_map_cancel_underground')) return Promise.resolve()
        if (pending) return new Promise(() => undefined)
        if (fail) return Promise.reject(new Error('Generation fixture failed'))
        return Promise.resolve(volume())
    })
    host = document.createElement('div')
    host.style.cssText = 'position:relative;width:900px;height:650px'
    document.body.append(host)
    wrapper = mount(SeedMapUndergroundPreview, {
        attachTo: host,
        props: {
            query: {
                seed: '12345',
                edition: 'java',
                version: '26.3',
                dimension: 'overworld',
                x: 0,
                y: 16,
                z: 0,
            },
        },
    })
    await expect
        .poll(() => wrapper?.text(), { timeout: 30000 })
        .toMatch(/Loaded \d+ full-height chunks/)
    expect(wrapper.text()).toContain('other solid rock is hidden')
    expect(wrapper.text()).toContain('aquifers are omitted')
    expect(wrapper.text()).toContain('Target:')
    await userEvent.click(page.getByRole('checkbox', { name: 'Bedrock' }))
    expect(wrapper.text()).toContain('each 16 × 16 column covering the entire build height')
    await userEvent.click(page.getByRole('checkbox', { name: 'Surface layer' }))
    expect((wrapper.findAll('input')[1].element as HTMLInputElement).checked).toBe(false)
    await expect
        .poll(
            () =>
                backend.invoke.mock.calls.filter(([command]) =>
                    command.endsWith('seed_map_underground_volume'),
                ).length,
        )
        .toBeGreaterThan(1)
    const requests = backend.invoke.mock.calls
        .filter(([command]) => command.endsWith('seed_map_underground_volume'))
        .map(([, args]) => args.request)
    expect(requests.every((request) => request.chunk)).toBe(true)
    expect(new Set(requests.map((request) => `${request.x}:${request.y}:${request.z}`)).size).toBe(
        requests.length,
    )
    const canvas = wrapper.get('canvas').element as HTMLCanvasElement
    expect(canvas.width).toBeGreaterThan(1)
    await userEvent.click(page.elementLocator(canvas), { position: { x: 800, y: 500 } })
    await expect.poll(() => document.pointerLockElement).toBe(canvas)
    const generationCalls = () =>
        backend.invoke.mock.calls.filter(([command]) =>
            command.endsWith('seed_map_underground_volume'),
        ).length
    const callsBeforeWalking = generationCalls()
    window.dispatchEvent(new KeyboardEvent('keydown', { code: 'Space' }))
    await expect.poll(generationCalls, { timeout: 6000 }).toBeGreaterThan(callsBeforeWalking)
    window.dispatchEvent(new KeyboardEvent('keyup', { code: 'Space' }))
    await expect.poll(() => wrapper?.text()).not.toContain('Cancel generation')
    expect(wrapper.get('canvas').element).toBe(canvas)
    expect(wrapper.text()).not.toContain('Could not generate')
    const beforeMove = wrapper.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0]
    window.dispatchEvent(new KeyboardEvent('keydown', { code: 'Space' }))
    await expect
        .poll(() => wrapper?.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0])
        .not.toBe(beforeMove)
    document.exitPointerLock()
    await expect.poll(() => document.pointerLockElement).toBe(null)
    await new Promise((resolve) => setTimeout(resolve, 180))
    const afterUnlock = wrapper.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0]
    await new Promise((resolve) => setTimeout(resolve, 250))
    expect(wrapper.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0]).toBe(
        afterUnlock,
    )
    await userEvent.click(page.getByRole('checkbox', { name: 'Cave boundaries' }))
    expect((wrapper.findAll('input')[0].element as HTMLInputElement).checked).toBe(false)
    fail = true
    await expect.poll(() => wrapper?.text()).toContain('Your previous view is kept')
    expect(wrapper.get('canvas').element).toBe(canvas)
    expect(wrapper.text()).toMatch(/Loaded \d+ full-height chunks/)
    pending = true
    await userEvent.click(page.getByRole('button', { name: 'Retry region' }))
    await expect.poll(() => wrapper?.text()).toContain('Cancel generation')
    wrapper.unmount()
    wrapper = undefined
    expect(
        backend.invoke.mock.calls.some(([command]) =>
            command.endsWith('seed_map_cancel_underground'),
        ),
    ).toBe(true)
})
