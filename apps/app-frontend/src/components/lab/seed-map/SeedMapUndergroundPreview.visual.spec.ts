import '@/test/visual-styles.scss'

import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

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

it('loads textured cavity geometry, switches layers, keeps the view on failure and cancels pending generation on exit', async () => {
    const volume = () => {
        const result = new ArrayBuffer(16 + 64 ** 3)
        const header = new DataView(result)
        ;[1, 0, 64, 64].forEach((value, index) => header.setUint32(index * 4, value, true))
        const blocks = new Uint8Array(result, 16).fill(1)
        for (let x = 16; x < 48; x++)
            for (let z = 16; z < 48; z++)
                for (let y = 16; y < 48; y++) blocks[(x * 64 + z) * 64 + y] = 0
        blocks[(32 * 64 + 32) * 64 + 16] = 5
        blocks[(34 * 64 + 32) * 64 + 16] = 4
        blocks[(30 * 64 + 32) * 64 + 15] = 16
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
        .toContain('Loaded 64 × 64 × 64 blocks')
    expect(wrapper.text()).toContain('other solid rock is hidden')
    expect(wrapper.text()).toContain('aquifers are omitted')
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
    const afterUnlock = wrapper.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0]
    await new Promise((resolve) => setTimeout(resolve, 250))
    expect(wrapper.text().match(/X -?\d+ · Y -?\d+ · Z -?\d+ · \d+ blocks\/s/)?.[0]).toBe(
        afterUnlock,
    )
    await userEvent.click(page.getByRole('checkbox', { name: 'Cave boundaries' }))
    expect((wrapper.findAll('input')[0].element as HTMLInputElement).checked).toBe(false)
    fail = true
    await userEvent.click(page.getByRole('button', { name: 'Generate around camera' }))
    await expect.poll(() => wrapper?.text()).toContain('Your previous view is kept')
    expect(wrapper.get('canvas').element).toBe(canvas)
    expect(wrapper.text()).toContain('Loaded 64 × 64 × 64 blocks')
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
