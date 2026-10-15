import { flushPromises, mount } from '@vue/test-utils'
import { IntlMessageFormat } from 'intl-messageformat'
import { afterEach, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import { analyzeMeta, cleanMeta } from '@/helpers/storage'

import MetaStorageCleanup from './MetaStorageCleanup.vue'

vi.mock('@/helpers/storage', () => ({ analyzeMeta: vi.fn(), cleanMeta: vi.fn() }))
vi.mock('@modrinth/ui', () => ({
    Admonition: defineComponent({
        setup:
            (_, { slots }) =>
            () =>
                h('div', slots.default?.()),
    }),
    StyledInput: defineComponent({ setup: () => () => h('input') }),
    Combobox: defineComponent({ props: ['options', 'modelValue'], setup: () => () => h('select') }),
    useFormatBytes: () => (size: number) => `${size} bytes`,
    Button: defineComponent({
        setup:
            (_, { attrs, slots }) =>
            () =>
                h('button', attrs, slots.default?.()),
    }),
    Checkbox: defineComponent({
        props: ['modelValue', 'disabled', 'description'],
        emits: ['update:modelValue'],
        setup:
            (props, { emit }) =>
            () =>
                h('button', {
                    disabled: props.disabled,
                    role: 'checkbox',
                    'aria-label': props.description,
                    'aria-checked': props.modelValue,
                    onClick: () => emit('update:modelValue', !props.modelValue),
                }),
    }),
    NewModal: defineComponent({
        name: 'NewModal',
        props: ['onHide', 'disableClose'],
        setup(props, { slots, expose }) {
            const visible = ref(false)
            expose({
                show: () => {
                    visible.value = true
                },
                hide: () => {
                    if (!props.disableClose) {
                        visible.value = false
                        props.onHide?.()
                    }
                },
            })
            return () =>
                visible.value ? h('section', [slots.default?.(), slots.actions?.()]) : null
        },
    }),
    defineMessages: (messages: unknown) => messages,
    useVIntl: () => ({
        formatMessage: (message: { defaultMessage: string }, values?: Record<string, number>) =>
            new IntlMessageFormat(message.defaultMessage, 'en').format(values),
    }),
}))

const mounted: ReturnType<typeof mount>[] = []
afterEach(() => {
    mounted.splice(0).forEach((wrapper) => wrapper.unmount())
    vi.resetAllMocks()
})
function button(wrapper: ReturnType<typeof mount>, text: string) {
    return wrapper.findAll('button').find((element) => element.text() === text)!
}
async function analyzed() {
    vi.mocked(analyzeMeta).mockResolvedValue({
        id: 'plan',
        scanned_at: '2026-10-15T00:00:00Z',
        warnings: [],
        groups: [
            {
                id: 'libraries',
                category: 'libraries',
                name: '',
                used: 30,
                unused: 12,
                unknown: 0,
                files: 4,
            },
            {
                id: 'java_versions/java',
                category: 'java_versions',
                name: 'java',
                used: 0,
                unused: 0,
                unknown: 50,
                files: 2,
            },
        ],
    })
    const refreshStorage = vi.fn().mockResolvedValue(true)
    const wrapper = mount(MetaStorageCleanup, { props: { refreshStorage } })
    mounted.push(wrapper)
    await button(wrapper, 'Analyze unused metadata').trigger('click')
    await flushPromises()
    return { wrapper, refreshStorage }
}

it('selects only unused groups and keeps confirmation open through deletion and refresh', async () => {
    const { wrapper, refreshStorage } = await analyzed()
    expect(wrapper.findAll('[role="checkbox"]')).toHaveLength(1)
    await wrapper.get('[role="checkbox"]').trigger('click')
    await button(wrapper, 'Clean selected files').trigger('click')
    expect(cleanMeta).not.toHaveBeenCalled()
    let finish!: (result: Awaited<ReturnType<typeof cleanMeta>>) => void
    vi.mocked(cleanMeta).mockReturnValue(
        new Promise((resolve) => {
            finish = resolve
        }),
    )
    await wrapper
        .findAll('button')
        .filter((element) => element.text() === 'Clean selected files')[1]
        .trigger('click')
    await flushPromises()
    expect(cleanMeta).toHaveBeenCalledWith('plan', ['libraries'])
    expect(wrapper.getComponent({ name: 'NewModal' }).props('disableClose')).toBe(true)
    finish({ removed_files: 2, removed_bytes: 12, skipped_files: 1, failures: [] })
    await flushPromises()
    expect(refreshStorage).toHaveBeenCalledOnce()
    expect(wrapper.find('section').exists()).toBe(false)
    expect(wrapper.text()).toContain('Deleted 2 files (12 bytes)')
})

it('keeps the reviewed selection and modal after a cleanup failure', async () => {
    const { wrapper } = await analyzed()
    await wrapper.get('[role="checkbox"]').trigger('click')
    await button(wrapper, 'Clean selected files').trigger('click')
    vi.mocked(cleanMeta).mockRejectedValue(new Error('references changed'))
    await wrapper
        .findAll('button')
        .filter((element) => element.text() === 'Clean selected files')[1]
        .trigger('click')
    await flushPromises()
    expect(wrapper.find('section').exists()).toBe(true)
    expect(wrapper.get('[role="checkbox"]').attributes('aria-checked')).toBe('true')
    expect(wrapper.text()).toContain('Cleanup could not finish')
    expect(wrapper.getComponent({ name: 'NewModal' }).props('disableClose')).toBe(false)
})

it('retries only the totals refresh after cleanup has already succeeded', async () => {
    const { wrapper, refreshStorage } = await analyzed()
    await wrapper.get('[role="checkbox"]').trigger('click')
    await button(wrapper, 'Clean selected files').trigger('click')
    refreshStorage.mockResolvedValueOnce(false)
    vi.mocked(cleanMeta).mockResolvedValue({
        removed_files: 2,
        removed_bytes: 12,
        skipped_files: 0,
        failures: [],
    })
    await wrapper
        .findAll('button')
        .filter((element) => element.text() === 'Clean selected files')[1]
        .trigger('click')
    await flushPromises()
    expect(wrapper.find('section').exists()).toBe(true)
    expect(wrapper.text()).toContain('Cleanup finished, but storage totals could not be refreshed')
    expect(cleanMeta).toHaveBeenCalledOnce()
    await button(wrapper, 'Retry').trigger('click')
    await flushPromises()
    expect(refreshStorage).toHaveBeenCalledTimes(2)
    expect(cleanMeta).toHaveBeenCalledOnce()
    expect(wrapper.find('section').exists()).toBe(false)
})
