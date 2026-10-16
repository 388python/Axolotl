import { Channel, invoke } from '@tauri-apps/api/core'

import type {
    AnalysisEvent,
    AnalysisSource,
    ModelMessage,
    ModelTurn,
    PreparedSession,
    ToolRequest,
    UploadTicket,
} from './types'

export const prepareSession = (instanceId: string, runId: string, source: AnalysisSource) =>
    invoke<PreparedSession>('plugin:logs|logs_logagent_prepare', { instanceId, runId, source })

export const cancelSession = (runId: string) =>
    invoke<void>('plugin:logs|logs_logagent_cancel', { runId })

export const modelTurn = (runId: string, turn: ModelTurn) =>
    invoke<ModelMessage>('plugin:logs|logs_logagent_model_turn', { runId, turn })

export const executeTool = (runId: string, request: ToolRequest) =>
    invoke<unknown>('plugin:logs|logs_logagent_tool', { runId, request })

export function runCloud(
    runId: string,
    ticket: UploadTicket | null,
    onEvent: (event: AnalysisEvent) => void,
) {
    const channel = new Channel<AnalysisEvent>()
    channel.onmessage = onEvent
    return invoke<{ content: string; ticket: UploadTicket | null }>(
        'plugin:logs|logs_logagent_cloud',
        { runId, ticket: ticket ? { ...ticket, success: true } : null, onEvent: channel },
    )
}
