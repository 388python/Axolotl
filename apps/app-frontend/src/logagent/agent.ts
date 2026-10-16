import { tool } from '@langchain/core/tools'
import { createDeepAgent, registerHarnessProfile } from 'deepagents/browser'
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
    const agent = createDeepAgent({
        model: new LogAgentModel(gateway),
        tools,
        systemPrompt: LOGAGENT_PROMPT,
        checkpointer: false,
        subagents: [],
    })
    const result = await agent.invoke(
        {
            messages: [
                { role: 'user', content: `User language: ${session.locale}\n${session.context}` },
            ],
        },
        { signal, recursionLimit: 34, callbacks: [], maxConcurrency: 1 },
    )
    const last = result.messages.at(-1)
    if (!last || typeof last.content !== 'string' || !last.content.trim())
        throw new Error('LogAgent returned no diagnostic text')
    return last.content
}
