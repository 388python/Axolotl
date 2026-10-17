import '@/test/visual-styles.scss'

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import SeedMapStructureDetails from './SeedMapStructureDetails.vue'

const calls = vi.hoisted(() => ({ details: vi.fn(), loot: vi.fn() }))
const language = vi.hoisted(() => ({ current: 'en-US' }))
vi.mock('@/lab/seed-map/advanced', () => ({
    getSeedMapStructureDetails: calls.details,
    getSeedMapContainerLoot: calls.loot,
}))
vi.mock('@/lab/recipe-generator/item-images', () => ({ loadMinecraftItemImage: vi.fn() }))
vi.mock('@modrinth/ui', async () => {
    const { ref } = await import('vue')
    const { IntlMessageFormat } = await import('intl-messageformat')
    const en = (await import('@/locales/en-US/index.json')).default as Record<
        string,
        { message: string }
    >
    const zh = (await import('@/locales/zh-CN/index.json')).default as Record<
        string,
        { message: string }
    >
    return {
        Button: (
            await import('../../../../../../packages/ui/src/components/base/buttons/Button.vue')
        ).default,
        Combobox: (await import('../../../../../../packages/ui/src/components/base/Combobox.vue'))
            .default,
        defineMessages: (messages: unknown) => messages,
        useVIntl: () => ({
            locale: ref(language.current),
            formatMessage: (
                message: { id: string; defaultMessage: string },
                values?: Record<string, string | number>,
            ) =>
                String(
                    new IntlMessageFormat(
                        (language.current === 'zh-CN' ? zh : en)[message.id]?.message ??
                            message.defaultMessage,
                        language.current,
                    ).format(values),
                ),
        }),
    }
})

const query = {
    seed: '1',
    edition: 'java' as const,
    version: '26.3',
    dimension: 'overworld' as const,
    x: 0,
    z: 0,
    featureMask: 1,
}
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => {
    wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
    vi.resetAllMocks()
    language.current = 'en-US'
})

function render() {
    if (!document.getElementById('teleports')) {
        const target = document.createElement('div')
        target.id = 'teleports'
        document.body.append(target)
    }
    const wrapper = mount(SeedMapStructureDetails, {
        props: { query, kind: 'village', title: 'Village' },
        attachTo: document.body,
    })
    wrappers.push(wrapper)
    ;(wrapper.element as HTMLElement).style.cssText =
        'width: 430px; height: 720px; background: var(--surface-2); color: var(--color-text-primary)'
    return wrapper
}

it('keeps a newer structure selection when an older calculation finishes later', async () => {
    let resolveOld!: (value: unknown) => void
    calls.details
        .mockImplementationOnce(
            () =>
                new Promise((resolve) => {
                    resolveOld = resolve
                }),
        )
        .mockResolvedValueOnce({
            supported: true,
            pieces: [],
            containers: [],
            truncated: false,
            limits: [],
            variant: { rotation: 90, mirror: 0, start: 3 },
        })
    const wrapper = render()
    await wrapper.setProps({ query: { ...query, x: 128 } })
    await vi.waitFor(() => expect(wrapper.text()).toContain('90°'))
    resolveOld({
        supported: true,
        pieces: [],
        containers: [],
        truncated: false,
        limits: [],
        variant: { rotation: 180, mirror: 0, start: 1 },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.text()).toContain('90°')
    expect(wrapper.text()).not.toContain('180°')
})

it('renders interactive 3D bounds and keeps map selection synchronized without inventing heights', async () => {
    calls.details.mockResolvedValue({
        supported: true,
        truncated: false,
        limits: ['relative_height'],
        containers: [],
        pieces: [
            {
                id: 0,
                name: 'SHPR',
                positionKnown: true,
                rotation: 0,
                position: { x: -20, y: 64, z: 0 },
                min: { x: -20, y: 64, z: 0 },
                max: { x: -10, y: 72, z: 15 },
            },
            {
                id: 1,
                name: 'SHLi',
                positionKnown: true,
                rotation: 90,
                position: { x: -10, y: 70, z: 0 },
                min: { x: -10, y: 70, z: 0 },
                max: { x: 5, y: 82, z: 10 },
            },
        ],
    })
    const wrapper = render()
    await wrapper.setProps({ kind: 'stronghold', title: 'Stronghold', selected: 1 })
    await flushPromises()
    const card = wrapper.get('[data-testid="seed-map-structure-capabilities"]')
    expect(card.text()).toContain('Supported')
    expect(card.text()).toContain('Not supported')
    expect(card.findAll('div')[1].text()).toContain('Verified world heights')
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    const canvas = wrapper.get('canvas').element as HTMLCanvasElement
    await vi.waitFor(() => expect(wrapper.get('canvas').attributes('data-ready')).toBe('true'))
    const gl = canvas.getContext('webgl2')!
    const pixels = () => {
        const data = new Uint8Array(canvas.width * canvas.height * 4)
        gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, data)
        return data
    }
    await vi.waitFor(() => expect(pixels().some((value) => value > 0)).toBe(true))
    expect(wrapper.get('[data-piece-id="1"]').attributes('aria-pressed')).toBe('true')
    const before = pixels()
    canvas.dispatchEvent(
        new WheelEvent('wheel', {
            deltaY: -200,
            clientX: 200,
            clientY: 400,
            bubbles: true,
            cancelable: true,
        }),
    )
    await vi.waitFor(() => expect(pixels()).not.toEqual(before))
    const zoomed = pixels()
    await userEvent.dragAndDrop(
        canvas,
        wrapper.get('[data-testid="seed-map-layout-preview"] strong').element,
    )
    await vi.waitFor(() => expect(pixels()).not.toEqual(zoomed))
    await wrapper.get('[data-piece-id="0"]').trigger('click')
    expect(wrapper.emitted('focus')?.at(-1)?.[0]).toMatchObject({ id: 0 })
    expect(wrapper.text()).toContain('Vertical relationships are relative')
    if (import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE)
        await page.screenshot({
            element: wrapper.element,
            path: `${import.meta.env.VITE_AXOLOTL_VISUAL_EVIDENCE}/seed-map-layout-3d.png`,
        })
})

it('retains a selectable horizontal fallback after a WebGL context loss', async () => {
    calls.details.mockResolvedValue({
        supported: true,
        truncated: false,
        limits: ['height_placeholder'],
        containers: [],
        pieces: [
            {
                id: 0,
                name: 'SHPR',
                positionKnown: true,
                rotation: 0,
                position: { x: 0, y: 64, z: 0 },
                min: { x: 0, y: 64, z: 0 },
                max: { x: 10, y: 72, z: 15 },
            },
        ],
    })
    const wrapper = render()
    await wrapper.setProps({ kind: 'stronghold' })
    await flushPromises()
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    await vi.waitFor(() => expect(wrapper.get('canvas').attributes('data-ready')).toBe('true'))
    await wrapper.get('canvas').trigger('webglcontextlost')
    expect(wrapper.text()).toContain('3D rendering is unavailable')
    await wrapper.get('svg[aria-label] rect').trigger('click')
    expect(wrapper.emitted('focus')?.at(-1)?.[0]).toMatchObject({ id: 0 })
    expect(wrapper.text()).toContain('Retry 3D preview')
})

it('loads loot only after opening its tab and reports unavailable piece positions', async () => {
    calls.details.mockResolvedValue({
        supported: true,
        pieces: [
            {
                id: 0,
                name: 'tent',
                positionKnown: false,
                position: { x: 0, y: 0, z: 0 },
                rotation: 0,
            },
        ],
        containers: [{ id: 0, piece: 0, x: 232, z: -392, table: 'barrels/abandoned_camp_barrel' }],
        truncated: false,
        limits: ['piece_positions_unavailable'],
    })
    calls.loot.mockResolvedValue({
        supported: true,
        items: [],
        unresolvedTables: ['missing/table'],
        limits: [],
    })
    const wrapper = render()
    await vi.waitFor(() => expect(wrapper.text()).toContain('no piece positions'))
    expect(calls.loot).not.toHaveBeenCalled()
    await wrapper
        .findAll('[role="tab"]')
        .find((tab) => tab.text() === 'Layout')!
        .trigger('click')
    expect(wrapper.find('svg[aria-label]').exists()).toBe(false)
    await wrapper
        .findAll('[role="tab"]')
        .find((tab) => tab.text() === 'Loot')!
        .trigger('click')
    await vi.waitFor(() => expect(calls.loot).toHaveBeenCalledOnce())
    expect(wrapper.text()).toContain('Unrecognized loot table')
    expect(wrapper.text()).not.toContain('missing/table')
    await wrapper.get('details summary').trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('missing/table'))
    expect(wrapper.text()).toContain('chest slot positions are not reconstructed')
})

it.each(['en-US', 'zh-CN'])(
    'uses translated names for layouts, loot and effects in %s while retaining raw identifiers on demand',
    async (locale) => {
        language.current = locale
        calls.details.mockResolvedValue({
            supported: true,
            truncated: false,
            limits: [],
            variant: { start: 255, rotation: 90, mirror: 0 },
            pieces: [
                {
                    id: 0,
                    name: 'SHPR',
                    positionKnown: true,
                    position: { x: 12, y: 64, z: 34 },
                    rotation: 90,
                },
            ],
            containers: [{ id: 0, piece: 0, x: 13, z: 35, table: 'chests/stronghold_library' }],
        })
        calls.loot.mockResolvedValue({
            supported: true,
            truncated: false,
            limits: [],
            items: [
                {
                    id: 'minecraft:diamond',
                    count: 2,
                    enchantments: [{ id: 'minecraft:fortune', level: 3 }],
                    effect: 'minecraft:regeneration',
                    duration: 160,
                },
            ],
        })
        const wrapper = render()
        await wrapper.setProps({ kind: 'stronghold' })
        await flushPromises()
        expect(wrapper.text()).not.toContain('255')
        await wrapper.findAll('[role="tab"]')[1].trigger('click')
        expect(wrapper.text()).toContain(locale === 'zh-CN' ? '末地传送门房间' : 'End portal room')
        expect(wrapper.text()).not.toContain('SHPR')
        expect(wrapper.get('[data-piece-id="0"]').text()).toContain(
            locale === 'zh-CN' ? '末地传送门房间' : 'End portal room',
        )
        await wrapper.get('[data-piece-id="0"]').trigger('click')
        expect(wrapper.emitted('focus')?.[0]?.[0]).toMatchObject({
            name: 'SHPR',
            position: { x: 12, y: 64, z: 34 },
        })
        await wrapper.findAll('[role="tab"]')[2].trigger('click')
        await vi.waitFor(() => expect(calls.loot).toHaveBeenCalled())
        await flushPromises()
        expect(wrapper.text()).toContain(
            locale === 'zh-CN' ? '要塞图书馆宝箱' : 'Stronghold library chest',
        )
        expect(wrapper.text()).toContain(locale === 'zh-CN' ? '钻石 ×2' : 'Diamond ×2')
        expect(wrapper.text()).toContain(locale === 'zh-CN' ? '时运 III' : 'Fortune III')
        expect(wrapper.text()).toContain(
            locale === 'zh-CN' ? '生命恢复 · 0:08' : 'Regeneration · 0:08',
        )
        expect(wrapper.text()).not.toContain('minecraft:fortune')
        await wrapper.get('details summary').trigger('click')
        await vi.waitFor(() => expect(wrapper.text()).toContain('SHPR'))
        expect(wrapper.text()).toContain('minecraft:fortune')
    },
)

it('names numbered portal variants and mirror state and resolves camp placeholders', async () => {
    language.current = 'zh-CN'
    calls.details.mockResolvedValue({
        supported: true,
        truncated: false,
        limits: [],
        pieces: [],
        containers: [],
        variant: { start: 3, giant: true, rotation: 90, mirror: 1 },
    })
    const wrapper = render()
    await wrapper.setProps({ kind: 'ruined-portal' })
    await flushPromises()
    expect(wrapper.text()).toContain('巨型废弃传送门 3')
    expect(wrapper.text()).toContain('giant_ruined_portal_3')
    expect(wrapper.text()).toContain('已镜像')
    calls.details.mockResolvedValue({
        supported: true,
        truncated: false,
        limits: ['piece_positions_unavailable'],
        containers: [],
        variant: { start: 7, biome: 188, rotation: 0, mirror: 0 },
        pieces: [
            {
                id: 0,
                name: 'abandoned_camp/tent/%s/tent_%s_7',
                positionKnown: false,
                position: { x: 0, y: 0, z: 0 },
                rotation: 0,
            },
        ],
    })
    await wrapper.setProps({ kind: 'abandoned-camp', query: { ...query, x: 64 } })
    await flushPromises()
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    expect(wrapper.text()).toContain('帐篷 7')
    expect(wrapper.text()).not.toContain('%s')
    expect(wrapper.text()).not.toContain('无镜像')
})
