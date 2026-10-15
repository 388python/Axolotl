import { flushPromises, mount } from '@vue/test-utils'
import { IntlMessageFormat } from 'intl-messageformat'
import { afterEach, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import { purgeLauncherCache } from '@/helpers/cache.js'

import CacheCleanupButton from './CacheCleanupButton.vue'
const preferences = vi.hoisted(() => ({ skipWarnings: false }))
vi.mock('@/helpers/cache.js', () => ({ purgeLauncherCache: vi.fn() }))
vi.mock('@/store/theme', () => ({
    useTheming: () => ({ getFeatureFlag: () => preferences.skipWarnings }),
}))
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
    preferences.skipWarnings = false
    vi.resetAllMocks()
})
it('keeps cache cleanup pending until the shared cleanup operation completes', async () => {
    const wrapper = mount(CacheCleanupButton, { props: { compact: true } })
    mounted.push(wrapper)
    await wrapper.get('button').trigger('click')
    expect(purgeLauncherCache).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('Game files, saves and backups are kept')
    let complete!: () => void
    vi.mocked(purgeLauncherCache).mockReturnValueOnce(
        new Promise<void>((resolve) => {
            complete = resolve
        }),
    )
    const confirm = wrapper.findAll('button').filter((button) => button.text() === 'Clear cache')[1]
    await confirm.trigger('click')
    await flushPromises()
    expect(wrapper.getComponent({ name: 'NewModal' }).props('disableClose')).toBe(true)
    complete()
    await flushPromises()
    expect(wrapper.find('section').exists()).toBe(false)
    expect(wrapper.emitted('cleared')).toHaveLength(1)
    expect(wrapper.get('button').text()).toBe('Cache cleared')
})
it('leaves the cache confirmation available for retry after failure', async () => {
    const wrapper = mount(CacheCleanupButton)
    mounted.push(wrapper)
    await wrapper.get('button').trigger('click')
    vi.mocked(purgeLauncherCache).mockRejectedValueOnce(new Error('cache unavailable'))
    await wrapper
        .findAll('button')
        .filter((button) => button.text() === 'Clear cache')[1]
        .trigger('click')
    await flushPromises()
    expect(wrapper.find('section').exists()).toBe(true)
    expect(wrapper.text()).toContain('Could not clear the cache')
    expect(wrapper.emitted('cleared')).toBeUndefined()
})
it('respects the preference to skip optional warnings', async () => {
    preferences.skipWarnings = true
    vi.mocked(purgeLauncherCache).mockResolvedValueOnce(undefined)
    const wrapper = mount(CacheCleanupButton)
    mounted.push(wrapper)
    await wrapper.get('button').trigger('click')
    await flushPromises()
    expect(purgeLauncherCache).toHaveBeenCalledTimes(1)
    expect(wrapper.find('section').exists()).toBe(false)
    expect(wrapper.emitted('cleared')).toHaveLength(1)
})
