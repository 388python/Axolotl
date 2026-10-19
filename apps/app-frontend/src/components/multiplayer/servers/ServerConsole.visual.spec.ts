import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'

import type { ServerView } from '@/composables/useServers'

import ServerConsole from './ServerConsole.vue'

const backend = vi.hoisted(() => ({ hydrate: vi.fn(async () => {}), unsubscribe: vi.fn() }))
vi.mock('@/composables/useServers', () => ({
    hydrateLog: backend.hydrate,
    subscribeServerConsoleOutput: () => backend.unsubscribe,
    useServers: () => ({ logLines: {}, sendCommand: vi.fn() }),
}))
vi.mock('@/helpers/servers', () => ({ servers: {} }))
vi.mock('@modrinth/ui', async () => {
    const { shallowRef } = await import('vue')
    return {
        ConsolePageLayout: {
            setup(_: unknown, { expose }: { expose: (value: unknown) => void }) {
                expose({ scrollToBottom: vi.fn() })
                return () => null
            },
        },
        JLineCommandInput: { render: () => null },
        createConsoleState: () => ({
            output: shallowRef([]),
            clear: vi.fn(),
            addLegacyLog: vi.fn(),
        }),
        defineMessages: (value: unknown) => value,
        provideConsoleManager: vi.fn(),
        useVIntl: () => ({ formatMessage: () => '' }),
    }
})

let wrapper: ReturnType<typeof mount> | undefined
const server = { id: 'server', running: true, serverType: 'vanilla' } as ServerView
afterEach(() => {
    wrapper?.unmount()
    wrapper = undefined
    vi.useRealTimers()
    backend.hydrate.mockReset().mockResolvedValue()
    backend.unsubscribe.mockClear()
})

it('does not start background polling when initial hydration completes after unmount', async () => {
    vi.useFakeTimers()
    let finish!: () => void
    backend.hydrate.mockReturnValueOnce(
        new Promise((resolve) => {
            finish = resolve
        }),
    )
    wrapper = mount(ServerConsole, { props: { server } })
    wrapper.unmount()
    wrapper = undefined
    finish()
    await flushPromises()
    await vi.advanceTimersByTimeAsync(3000)
    expect(backend.hydrate).toHaveBeenCalledOnce()
    expect(backend.unsubscribe).toHaveBeenCalledOnce()
})

it('waits for a slow reconciliation and never schedules another one after unmount', async () => {
    vi.useFakeTimers()
    wrapper = mount(ServerConsole, { props: { server } })
    await flushPromises()
    let finish!: () => void
    backend.hydrate.mockReturnValueOnce(
        new Promise((resolve) => {
            finish = resolve
        }),
    )
    await vi.advanceTimersByTimeAsync(1000)
    await vi.advanceTimersByTimeAsync(5000)
    expect(backend.hydrate).toHaveBeenCalledTimes(2)
    wrapper.unmount()
    wrapper = undefined
    finish()
    await flushPromises()
    await vi.advanceTimersByTimeAsync(3000)
    expect(backend.hydrate).toHaveBeenCalledTimes(2)
})

it('resumes reconciliation when a stopped server starts again', async () => {
    vi.useFakeTimers()
    wrapper = mount(ServerConsole, { props: { server } })
    await flushPromises()
    await wrapper.setProps({ server: { ...server, running: false } })
    await vi.advanceTimersByTimeAsync(2000)
    expect(backend.hydrate).toHaveBeenCalledOnce()
    await wrapper.setProps({ server })
    await flushPromises()
    expect(backend.hydrate).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(1000)
    expect(backend.hydrate).toHaveBeenCalledTimes(3)
})
