import assert from 'node:assert/strict'
import { test } from 'node:test'

import { runLocalAgent } from './agent.ts'
import type { AgentGateway, ModelTurn } from './types.ts'

const session = {
    run_id: 'test',
    provider_id: 'test',
    model_id: 'test',
    locale: 'zh-CN',
    context: 'A crash occurred.',
}

test('Deep Agents uses only diagnostic tools and preserves tool IDs and opaque protocol state', async () => {
    const turns: ModelTurn[] = []
    const used: string[] = []
    const gateway: AgentGateway = {
        async modelTurn(turn) {
            turns.push(turn)
            assert.deepEqual(
                turn.tools.map((tool) => (tool.function as { name: string }).name).sort(),
                ['grep_log_file', 'list_log_files', 'read_log_file'],
            )
            if (turns.length === 1)
                return {
                    role: 'assistant',
                    content: '',
                    tool_calls: [
                        {
                            id: 'call-123',
                            name: 'read_log_file',
                            args: { file_id: 'log-1', start_line: 200 },
                        },
                    ],
                    opaque: { signature: 'provider-required' },
                }
            assert.equal(
                turn.messages.find((message) => message.role === 'tool')?.tool_call_id,
                'call-123',
            )
            assert.deepEqual(
                turn.messages.find((message) => message.role === 'assistant')?.opaque,
                { signature: 'provider-required' },
            )
            return {
                role: 'assistant',
                content: 'The log identifies an incompatible Java version.',
            }
        },
        async tool(request) {
            assert.equal(request.file_id, 'log-1')
            return { line: 200, text: 'UnsupportedClassVersionError' }
        },
    }
    const output = await runLocalAgent(session, gateway, new AbortController().signal, (tool) =>
        used.push(tool),
    )
    assert.equal(turns.length, 2)
    assert.deepEqual(used, ['read_log_file'])
    assert.match(output, /Java/)
})

test('an aborted run does not call the model', async () => {
    const controller = new AbortController()
    controller.abort()
    const gateway: AgentGateway = {
        modelTurn: async () => {
            throw new Error('must not run')
        },
        tool: async () => {
            throw new Error('must not run')
        },
    }
    await assert.rejects(runLocalAgent(session, gateway, controller.signal, () => {}))
})
