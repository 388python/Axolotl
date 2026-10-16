import { computed, onScopeDispose, ref } from 'vue'

import * as backend from '../logagent/backend.ts'
import type {
    AgentGateway,
    AnalysisEvent,
    AnalysisSource,
    PreparedSession,
    UploadTicket,
} from '../logagent/types.ts'

interface Dependencies {
    prepare: typeof backend.prepareSession
    cancel: typeof backend.cancelSession
    cloud: typeof backend.runCloud
    local: (
        session: PreparedSession,
        gateway: AgentGateway,
        signal: AbortSignal,
        onTool: (name: string) => void,
    ) => Promise<string>
    gateway: (runId: string) => AgentGateway
}

const dependencies: Dependencies = {
    prepare: backend.prepareSession,
    cancel: backend.cancelSession,
    cloud: backend.runCloud,
    local: async (...args) => (await import('../logagent/agent.ts')).runLocalAgent(...args),
    gateway: (runId) => ({
        modelTurn: (turn) => backend.modelTurn(runId, turn),
        tool: (request) => backend.executeTool(runId, request),
    }),
}

export function useLogAgent(deps: Dependencies = dependencies) {
    const status = ref<'idle' | 'preparing' | 'processing' | 'ready' | 'error' | 'cancelled'>(
        'idle',
    )
    const output = ref('')
    const error = ref('')
    const event = ref<AnalysisEvent | null>(null)
    const runSource = ref<AnalysisSource | null>(null)
    const model = ref('')
    const busy = computed(() => status.value === 'preparing' || status.value === 'processing')
    let active: { id: string; abort: AbortController } | null = null

    function cancel() {
        const run = active
        active = null
        if (!run) return
        run.abort.abort()
        status.value = 'cancelled'
        event.value = null
        void deps.cancel(run.id).catch(() => {})
    }

    function reset() {
        cancel()
        status.value = 'idle'
        output.value = ''
        error.value = ''
        event.value = null
        runSource.value = null
        model.value = ''
    }

    async function start(
        instanceId: string,
        source: AnalysisSource,
        ticket: UploadTicket | null | Promise<UploadTicket | null>,
        onUpload: (ticket: UploadTicket) => void,
    ) {
        if (busy.value) return
        reset()
        const run = { id: crypto.randomUUID(), abort: new AbortController() }
        active = run
        runSource.value = source
        status.value = 'preparing'
        const current = () => active === run && !run.abort.signal.aborted
        const receive = (next: AnalysisEvent) => {
            if (!current() || next.run_id !== run.id) return
            event.value = next
            if (next.event_type === 'delta' && typeof next.data.content === 'string')
                output.value += next.data.content
            if (next.event_type === 'uploaded' && typeof next.data.id === 'string')
                onUpload(next.data as unknown as UploadTicket)
        }
        try {
            const session = await deps.prepare(instanceId, run.id, source)
            if (!current()) return
            model.value = source === 'custom' ? `${session.provider_id} / ${session.model_id}` : ''
            status.value = 'processing'
            let text: string
            if (source === 'custom') {
                text = await deps.local(session, deps.gateway(run.id), run.abort.signal, (name) =>
                    receive({ run_id: run.id, event_type: 'tool', data: { name } }),
                )
            } else {
                const preparedTicket = await Promise.resolve(ticket).catch(() => null)
                if (!current()) return
                const result = await deps.cloud(run.id, preparedTicket, receive)
                text = result.content
            }
            if (!current()) return
            output.value = text
            event.value = null
            status.value = 'ready'
        } catch (cause) {
            if (!current()) return
            error.value =
                cause instanceof Error
                    ? cause.message
                    : typeof cause === 'string'
                      ? cause
                      : JSON.stringify(cause)
            event.value = null
            status.value = 'error'
        } finally {
            await deps.cancel(run.id).catch(() => {})
            if (active === run) active = null
        }
    }

    onScopeDispose(cancel, true)
    return { status, output, error, event, busy, runSource, model, start, cancel, reset }
}
