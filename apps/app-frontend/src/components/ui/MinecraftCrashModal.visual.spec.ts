import { mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'

import MinecraftCrashModal from './MinecraftCrashModal.vue'

const calls = vi.hoisted(() => ({
    logs: vi.fn(),
    prepare: vi.fn(),
    cancel: vi.fn(),
    process: undefined as
        | undefined
        | ((event: {
              instance_id: string
              uuid: string
              event: 'launched' | 'finished'
              crashed?: boolean
          }) => void),
}))

vi.mock('@modrinth/ui', () => ({
    defineMessages: (messages: unknown) => messages,
    commonMessages: { closeButton: { defaultMessage: 'Close' } },
    useVIntl: () => ({
        formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
    }),
    injectModrinthClient: () => ({}),
    injectNotificationManager: () => ({ addNotification: vi.fn() }),
    Button: defineComponent({
        setup:
            (_, { slots }) =>
            () =>
                h('button', slots.default?.()),
    }),
    Card: defineComponent({
        setup:
            (_, { slots }) =>
            () =>
                h('div', slots.default?.()),
    }),
    Combobox: defineComponent({ inheritAttrs: false, setup: () => () => h('select') }),
    LogViewport: defineComponent({ setup: () => () => h('div') }),
    ProgressBar: defineComponent({ setup: () => () => h('div') }),
    shareLogs: vi.fn(),
    NewModal: defineComponent({
        setup: (_, { slots, expose }) => {
            const visible = ref(false)
            expose({
                show: () => {
                    visible.value = true
                },
                hide: () => {
                    visible.value = false
                },
            })
            return () => (visible.value ? h('div', slots.default?.()) : null)
        },
    }),
}))
vi.mock('@modrinth/utils/highlightjs', () => ({ renderHighlightedString: (text: string) => text }))
vi.mock('@/components/ui/CrashModChangesModal.vue', () => ({
    default: defineComponent({ setup: () => () => null }),
}))
vi.mock('@/helpers/events.js', () => ({
    process_listener: async (listener: typeof calls.process) => {
        calls.process = listener
        return () => {}
    },
}))
vi.mock('@/helpers/ai', () => ({
    getAIState: async () => ({
        settings: { enabled: true },
        providers: [
            {
                provider_id: 'codeflow',
                enabled: true,
                models: [{ id: 'gpt-6-luna', enabled: true }],
            },
        ],
    }),
}))
vi.mock('@/helpers/instance', () => ({ get: async () => ({ name: 'Test' }) }))
vi.mock('@/composables/useCrashAnalysis', () => ({
    clearCrashAnalysis: vi.fn(),
    refreshCrashAnalysis: async () => null,
}))
vi.mock('@/helpers/logs', () => ({
    get_crash_analysis_ai_settings: async () => ({
        enabled: true,
        provider_id: 'codeflow',
        model_id: 'gpt-6-luna',
        ai_source: 'custom',
    }),
    get_log_share_settings: async () => ({
        ai_source: 'custom',
        auto_upload: false,
        show_progress: true,
    }),
    get_logs: calls.logs,
    get_output_by_filename: async () => 'log evidence',
    logshare_get_insights: vi.fn(),
    logshare_upload_crash: vi.fn(),
    record_shared_log: vi.fn(),
}))
vi.mock('@/logagent/backend.ts', () => ({
    prepareSession: calls.prepare,
    cancelSession: calls.cancel,
    runCloud: vi.fn(),
    modelTurn: vi.fn(),
    executeTool: vi.fn(),
}))

const cleanup: (() => void)[] = []
afterEach(() => {
    cleanup.splice(0).forEach((dispose) => dispose())
    vi.restoreAllMocks()
})

it('keeps the active LogAgent workspace when log enumeration and duplicate crash notifications arrive late', async () => {
    let resolveLogs!: (logs: unknown[]) => void
    let rejectPreparation!: (error: Error) => void
    calls.logs.mockImplementationOnce(
        () =>
            new Promise((resolve) => {
                resolveLogs = resolve
            }),
    )
    calls.prepare.mockImplementationOnce(
        () =>
            new Promise((_, reject) => {
                rejectPreparation = reject
            }),
    )
    calls.cancel.mockResolvedValue(undefined)
    const wrapper = mount(MinecraftCrashModal, { global: { directives: { tooltip: () => {} } } })
    cleanup.push(() => wrapper.unmount())
    const payload = {
        kind: 'minecraft_crash' as const,
        instance_id: 'instance',
        instance_name: 'Test',
    }
    calls.process?.({ instance_id: 'instance', uuid: 'run-1', event: 'launched' })
    await wrapper.vm.handleWarning(payload)
    const pending = wrapper.vm.openAIAnalysis()
    await nextTick()
    expect(wrapper.find('.crash-modal-ai-output').exists()).toBe(true)
    resolveLogs([{ filename: 'latest.log', log_type: 'InfoLog', age: 1000 }])
    await vi.waitFor(() => expect(wrapper.text()).toContain('latest.log'))
    expect(wrapper.find('.crash-modal-ai-output').exists()).toBe(true)
    const time = Date.now()
    vi.spyOn(Date, 'now').mockReturnValue(time + 20_000)
    await wrapper.vm.handleWarning(payload)
    expect(wrapper.find('.crash-modal-ai-output').exists()).toBe(true)
    expect(calls.logs).toHaveBeenCalledTimes(1)
    rejectPreparation(new Error('diagnostic preparation failed'))
    await pending
    await nextTick()
    expect(wrapper.find('.crash-modal-ai-output').exists()).toBe(true)
    expect(wrapper.text()).toContain('analysis failed')
    calls.process?.({ instance_id: 'instance', uuid: 'run-1', event: 'finished', crashed: true })
    await new Promise((resolve) => window.setTimeout(resolve, 2300))
    expect(wrapper.find('.crash-modal-ai-output').exists()).toBe(true)
    expect(wrapper.text()).toContain('analysis failed')
    expect(calls.logs).toHaveBeenCalledTimes(1)
})
