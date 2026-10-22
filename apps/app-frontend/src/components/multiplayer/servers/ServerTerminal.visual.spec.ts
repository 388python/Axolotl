import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type { Terminal } from '@xterm/xterm'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { ServerView } from '@/composables/useServers'
import type { ServerConsoleDelta } from '@/helpers/servers'

import ServerTerminal from './ServerTerminal.vue'

const backend = vi.hoisted(() => ({
    delta: vi.fn<(id: string, cursor?: unknown) => Promise<ServerConsoleDelta>>(),
    input: vi.fn(async (_id: string, _bytes: Uint8Array) => {}),
    resize: vi.fn(async (_id: string, _cols: number, _rows: number) => {}),
    error: vi.fn(),
    unsubscribe: vi.fn(),
    notify: undefined as (() => void) | undefined,
    terminal: undefined as Terminal | undefined,
}))
vi.mock('@/helpers/servers', () => ({
    servers: {
        getConsoleDelta: backend.delta,
        sendConsoleInput: backend.input,
        resizeConsole: backend.resize,
    },
    base64ToBytes: (data: string) => Uint8Array.from(atob(data), (char) => char.charCodeAt(0)),
}))
vi.mock('@/composables/useServers', () => ({
    subscribeServerConsoleOutput: (_id: string, notify: () => void) => {
        backend.notify = notify
        return backend.unsubscribe
    },
}))
vi.mock('@xterm/xterm', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@xterm/xterm')>()
    return {
        ...actual,
        Terminal: class extends actual.Terminal {
            constructor(options: ConstructorParameters<typeof actual.Terminal>[0]) {
                super(options)
                backend.terminal = this
            }
        },
    }
})
vi.mock('@modrinth/ui', async () => {
    const { defineComponent, h } = await import('vue')
    return {
        defineMessages: (messages: unknown) => messages,
        useVIntl: () => ({
            formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
        }),
        injectNotificationManager: () => ({ handleError: backend.error }),
        commonMessages: {
            searchPlaceholder: { defaultMessage: 'Search' },
            downloadButton: { defaultMessage: 'Download' },
        },
        Button: defineComponent({
            props: ['type'],
            setup:
                (_, { slots }) =>
                () =>
                    h('button', { type: 'button' }, slots.default?.()),
        }),
        StyledInput: defineComponent({
            props: ['modelValue', 'icon', 'wrapperClass', 'clearable'],
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
    }
})

let wrapper: VueWrapper | undefined
const server = { id: 'server', running: true, serverType: 'vanilla' } as ServerView
const encoder = new TextEncoder()
const decoder = new TextDecoder()
const delta = (text = '', sequence = 0, reset = false, epoch = 1): ServerConsoleDelta => ({
    data: btoa(String.fromCharCode(...encoder.encode(text))),
    cursor: { epoch, sequence },
    reset,
})
function screen() {
    const buffer = backend.terminal!.buffer.active
    return Array.from({ length: buffer.length }, (_, index) =>
        buffer.getLine(index)?.translateToString(true),
    ).join('\n')
}
function inputText() {
    return backend.input.mock.calls.map(([, bytes]) => decoder.decode(bytes)).join('')
}
async function mountTerminal() {
    wrapper = mount(ServerTerminal, {
        props: { server, view: 'terminal' },
        attachTo: document.body,
    })
    wrapper
        .get('div')
        .element.setAttribute(
            'style',
            'display:flex;width:800px;height:420px;flex-direction:column',
        )
    await vi.waitFor(() => expect(backend.delta).toHaveBeenCalled())
    await flushPromises()
    return wrapper
}
async function output(text: string, sequence = 1, reset = false, epoch = 1) {
    backend.delta.mockResolvedValue(delta(text, sequence, reset, epoch))
    const calls = backend.delta.mock.calls.length
    backend.notify?.()
    await vi.waitFor(() => expect(backend.delta.mock.calls.length).toBeGreaterThan(calls))
    backend.delta.mockResolvedValue(delta('', sequence, false, epoch))
    await flushPromises()
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
}

beforeEach(() => {
    vi.clearAllMocks()
    backend.delta.mockReset().mockResolvedValue(delta())
})
afterEach(() => {
    wrapper?.unmount()
    wrapper = undefined
    document.body.replaceChildren()
})

it('renders raw ANSI redraws, colors, Unicode and the alternate screen with real xterm', async () => {
    await mountTerminal()
    await output('\x1b[31mred\x1b[0m 中文\r\nprogress 10%\r\x1b[2Kprogress 90%')
    expect(screen()).toContain('red 中文')
    expect(screen()).toContain('progress 90%')
    expect(screen()).not.toContain('progress 10%')
    expect(backend.terminal!.buffer.active.getLine(0)!.getCell(0)!.getFgColor()).toBe(1)
    await output('\x1b[?1049h\x1b[Halternate', 2)
    expect(backend.terminal!.buffer.active.type).toBe('alternate')
    expect(screen()).toContain('alternate')
    await output('\x1b[?1049l', 3)
    expect(backend.terminal!.buffer.active.type).toBe('normal')
    expect(screen()).toContain('progress 90%')
})

it('forwards Enter, Tab, application cursor keys, Backspace and Ctrl+C without a custom command field', async () => {
    await mountTerminal()
    await output('\x1b[?1h> ')
    const field = wrapper!.get<HTMLTextAreaElement>('.xterm-helper-textarea')
    field.element.focus()
    for (const [key, keyCode, ctrlKey] of [
        ['Enter', 13, false],
        ['Tab', 9, false],
        ['ArrowUp', 38, false],
        ['Backspace', 8, false],
        ['c', 67, true],
    ] as const) {
        await field.trigger('keydown', { key, keyCode, ctrlKey })
        await field.trigger('keyup', { key, keyCode, ctrlKey })
    }
    await vi.waitFor(() => expect(inputText()).toBe('\r\t\x1bOA\x7f\x03'))
})

it('preserves bracketed multi-line Unicode paste and splits large input within the IPC limit', async () => {
    await mountTerminal()
    await output('\x1b[?2004h')
    const text = '中'.repeat(30000) + '\nsecond line'
    backend.terminal!.paste(text)
    await vi.waitFor(() => expect(backend.input).toHaveBeenCalledTimes(2))
    const bytes = new Uint8Array(
        backend.input.mock.calls.reduce((total, [, chunk]) => total + chunk.length, 0),
    )
    let offset = 0
    for (const [, chunk] of backend.input.mock.calls) {
        expect(chunk.length).toBeLessThanOrEqual(65536)
        bytes.set(chunk, offset)
        offset += chunk.length
    }
    expect(decoder.decode(bytes)).toBe(`\x1b[200~${text.replaceAll('\n', '\r')}\x1b[201~`)
})

it('responds to terminal device queries so interactive programs do not stall', async () => {
    await mountTerminal()
    await output('\x1b[6n')
    await vi.waitFor(() => expect(inputText()).toBe('\x1b[1;1R'))
})

it('keeps the last cursor after failure and retries missing output without clearing the display', async () => {
    await mountTerminal()
    await output('old', 10)
    backend.delta.mockRejectedValueOnce(new Error('temporarily unavailable'))
    backend.notify?.()
    await vi.waitFor(() => expect(backend.error).toHaveBeenCalledOnce())
    expect(screen()).toContain('old')
    await output('new', 11)
    expect(screen()).toContain('oldnew')
    expect(backend.delta).toHaveBeenLastCalledWith('server', { epoch: 1, sequence: 10 })
})

it('resizes the actual PTY, preserves the terminal when changing views and disables stopped input', async () => {
    await mountTerminal()
    await output('keep me', 1)
    await vi.waitFor(() => expect(backend.resize).toHaveBeenCalled())
    const instance = backend.terminal
    await wrapper!.setProps({ view: 'logs' })
    await wrapper!.setProps({ view: 'terminal' })
    expect(backend.terminal).toBe(instance)
    expect(screen()).toContain('keep me')
    await wrapper!.setProps({ server: { ...server, running: false } })
    backend.terminal!.input('blocked', true)
    await flushPromises()
    expect(backend.input).not.toHaveBeenCalled()
})

it('discards a previous server response and fetches the new session without its cursor', async () => {
    await mountTerminal()
    let finish!: (value: ServerConsoleDelta) => void
    backend.delta.mockReturnValueOnce(
        new Promise((resolve) => {
            finish = resolve
        }),
    )
    const calls = backend.delta.mock.calls.length
    backend.notify?.()
    await vi.waitFor(() => expect(backend.delta.mock.calls.length).toBeGreaterThan(calls))
    await wrapper!.setProps({ server: { ...server, id: 'other' } })
    backend.delta.mockResolvedValue(delta('other output', 1, true, 2))
    finish(delta('stale output', 99))
    await vi.waitFor(() => expect(screen()).toContain('other output'))
    expect(screen()).not.toContain('stale output')
    expect(backend.delta).toHaveBeenLastCalledWith('other', undefined)
})

it('starts a fresh polling session for preparation even when no output event is delivered', async () => {
    await mountTerminal()
    await wrapper!.setProps({ server: { ...server, running: false } })
    await output('previous run', 9)
    backend.delta.mockResolvedValue(delta('Preparing Fabric server files', 1, true, 2))
    const calls = backend.delta.mock.calls.length
    await wrapper!.setProps({ server: { ...server, running: false, starting: true } })
    await vi.waitFor(() => expect(backend.delta.mock.calls.length).toBeGreaterThan(calls))
    await vi.waitFor(() => expect(screen()).toContain('Preparing Fabric server files'))
    expect(screen()).not.toContain('previous run')
    expect(backend.delta).toHaveBeenLastCalledWith('server', undefined)
    expect(backend.terminal!.options.disableStdin).toBe(true)
})

it('unsubscribes and ignores a response that arrives after unmount', async () => {
    await mountTerminal()
    let finish!: (value: ServerConsoleDelta) => void
    backend.delta.mockReturnValueOnce(
        new Promise((resolve) => {
            finish = resolve
        }),
    )
    const calls = backend.delta.mock.calls.length
    backend.notify?.()
    await vi.waitFor(() => expect(backend.delta.mock.calls.length).toBeGreaterThan(calls))
    wrapper!.unmount()
    wrapper = undefined
    finish(delta('late response', 1))
    await flushPromises()
    expect(backend.unsubscribe).toHaveBeenCalledOnce()
    expect(backend.error).not.toHaveBeenCalled()
})
