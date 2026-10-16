import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AIMessage, ToolMessage } from '@langchain/core/messages'

import { diagnosticText, runLocalAgent } from './agent.ts'
import { LogAgentModel } from './model.ts'
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

test('accepts standard content blocks without displaying reasoning blocks', () => {
    const message = new AIMessage({
        contentBlocks: [
            { type: 'reasoning', reasoning: 'Internal reasoning' },
            { type: 'text', text: 'Java is incompatible.\n' },
            { type: 'text', text: 'Select a supported Java version.' },
        ],
    })
    assert.equal(diagnosticText(message), 'Java is incompatible.\nSelect a supported Java version.')
})

test('the complete agent loop accepts a final response normalized into content blocks', async (context) => {
    const generate = LogAgentModel.prototype._generate
    context.mock.method(
        LogAgentModel.prototype,
        '_generate',
        async function (this: LogAgentModel, ...args: Parameters<typeof generate>) {
            const result = await generate.apply(this, args)
            for (const generation of result.generations) {
                generation.message = new AIMessage({
                    ...generation.message,
                    contentBlocks: generation.message.contentBlocks,
                })
            }
            return result
        },
    )
    const output = await runLocalAgent(
        session,
        {
            modelTurn: async () => ({
                role: 'assistant',
                content: '诊断结果：Java 版本不兼容。',
                tool_calls: [],
            }),
            tool: async () => {
                throw new Error('No tool needed for this response')
            },
        },
        new AbortController().signal,
        () => {},
    )
    assert.equal(output, '诊断结果：Java 版本不兼容。')
})

test('does not treat tool output or unfinished tool calls as a final diagnosis', () => {
    assert.throws(
        () => diagnosticText(new ToolMessage({ content: 'log evidence', tool_call_id: 'call-1' })),
        /without a final assistant reply/,
    )
    assert.throws(
        () =>
            diagnosticText(
                new AIMessage({
                    content: 'Checking logs',
                    tool_calls: [{ id: 'call-1', name: 'list_log_files', args: {} }],
                }),
            ),
        /unresolved tool calls/,
    )
    assert.throws(
        () =>
            diagnosticText(
                new AIMessage({
                    contentBlocks: [{ type: 'reasoning', reasoning: 'Internal reasoning' }],
                }),
            ),
        /content: reasoning/,
    )
})
