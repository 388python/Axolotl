import '@/test/visual-styles.scss'

import { mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import SeedMapStructureDetails from './SeedMapStructureDetails.vue'

const calls = vi.hoisted(() => ({ details: vi.fn(), loot: vi.fn() }))
vi.mock('@/lab/seed-map/advanced', () => ({
    getSeedMapStructureDetails: calls.details,
    getSeedMapContainerLoot: calls.loot,
}))
vi.mock('@/lab/recipe-generator/item-images', () => ({ loadMinecraftItemImage: vi.fn() }))
vi.mock('@modrinth/ui', async () => ({
    Button: (await import('../../../../../../packages/ui/src/components/base/buttons/Button.vue'))
        .default,
    Combobox: (await import('../../../../../../packages/ui/src/components/base/Combobox.vue'))
        .default,
    defineMessages: (messages: unknown) => messages,
    useVIntl: () => ({
        locale: ref('en-US'),
        formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
    }),
}))

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
    expect(wrapper.text()).toContain('missing/table')
    expect(wrapper.text()).toContain('chest slot positions are not reconstructed')
})
