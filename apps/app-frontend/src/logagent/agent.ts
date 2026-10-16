import { AIMessage, type BaseMessage, ToolMessage } from '@langchain/core/messages'
import { tool } from '@langchain/core/tools'
import { createDeepAgent, registerHarnessProfile } from 'deepagents/browser'
import { createMiddleware } from 'langchain'
import { z } from 'zod'

import { LogAgentModel } from './model.ts'
import { LOGAGENT_PROMPT } from './prompts.ts'
import type { AgentGateway, PreparedSession } from './types.ts'

registerHarnessProfile('axolotl-logagent', {
    excludedTools: [
        'ls',
        'read_file',
        'write_file',
        'edit_file',
        'delete',
        'glob',
        'grep',
        'execute',
        'task',
        'write_todos',
    ],
    generalPurposeSubagent: { enabled: false },
})

export function diagnosticText(message: BaseMessage | undefined): string {
    if (!message || !AIMessage.isInstance(message))
        throw new Error('LogAgent finished without a final assistant reply')
    if (message.tool_calls?.length)
        throw new Error('LogAgent finished with unresolved tool calls instead of a diagnosis')
    const text = message.text
    if (!text.trim()) {
        const blocks = message.contentBlocks.map((block) => block.type).join(', ') || 'empty'
        throw new Error(`LogAgent final reply contains no diagnostic text (content: ${blocks})`)
    }
    return text
}

export async function runLocalAgent(
    session: PreparedSession,
    gateway: AgentGateway,
    signal: AbortSignal,
    onTool: (name: string) => void,
): Promise<string> {
    const invokeTool = async (name: string, request: Record<string, unknown>) => {
        signal.throwIfAborted()
        onTool(name)
        const result = await gateway.tool({ name, ...request })
        signal.throwIfAborted()
        return JSON.stringify(result)
    }
    const tools = [
        tool(() => invokeTool('list_log_files', {}), {
            name: 'list_log_files',
            description: 'List snapshot file IDs, names and original line ranges.',
            schema: z.object({}),
        }),
        tool((request) => invokeTool('read_log_file', request), {
            name: 'read_log_file',
            description:
                'Read at most 160 original numbered lines from a snapshot file. File paths are not accepted.',
            schema: z.object({
                file_id: z.string(),
                start_line: z.number().int().positive().optional(),
                end_line: z.number().int().positive().optional(),
            }),
        }),
        tool((request) => invokeTool('grep_log_file', request), {
            name: 'grep_log_file',
            description:
                'Case-insensitive literal search in one snapshot file, with merged surrounding context.',
            schema: z.object({
                file_id: z.string(),
                query: z.string().min(1).max(256),
                context: z.number().int().min(0).max(10).optional(),
            }),
        }),
    ]
    const availableTools = new Set(tools.map((tool) => tool.name))
    let invalidToolRounds = 0
    let modelTurns = 0
    const gatewayWithBudget: AgentGateway = {
        ...gateway,
        modelTurn: (turn) => {
            if (++modelTurns > 16) throw new Error('LogAgent reached the 16 model turn limit')
            return gateway.modelTurn(turn)
        },
    }
    const diagnosticTools = createMiddleware({
        name: 'DiagnosticToolValidation',
        afterModel: {
            canJumpTo: ['model'],
            hook: (state) => {
                const message = state.messages.at(-1)
                if (
                    !message ||
                    !AIMessage.isInstance(message) ||
                    !message.tool_calls?.some((call) => !availableTools.has(call.name))
                )
                    return
                const names = message.tool_calls
                    .filter((call) => !availableTools.has(call.name))
                    .map((call) => call.name)
                    .join(', ')
                if (++invalidToolRounds > 2)
                    throw new Error(`LogAgent repeatedly requested unavailable tools: ${names}`)
                return {
                    messages: message.tool_calls.map(
                        (call) =>
                            new ToolMessage({
                                tool_call_id: call.id ?? '',
                                name: call.name,
                                status: 'error',
                                content: `This tool batch was not executed: ${names} is not available. Use only list_log_files, read_log_file, grep_log_file, or finish with a diagnostic reply.`,
                            }),
                    ),
                    jumpTo: 'model' as const,
                }
            },
        },
    })
    const agent = createDeepAgent({
        model: new LogAgentModel(gatewayWithBudget),
        tools,
        systemPrompt: LOGAGENT_PROMPT,
        checkpointer: false,
        subagents: [],
        middleware: [diagnosticTools],
    })
    const result = await agent.invoke(
        {
            messages: [
                { role: 'user', content: `User language: ${session.locale}\n${session.context}` },
            ],
        },
        { signal, recursionLimit: 100, callbacks: [], maxConcurrency: 1 },
    )
    const last = result.messages.at(-1)
    if (last && AIMessage.isInstance(last) && last.tool_calls?.length) {
        throw new Error(
            `LogAgent stopped after ${modelTurns} model turns with unresolved tools: ${last.tool_calls.map((call) => call.name).join(', ')}`,
        )
    }
    return diagnosticText(last)
}
