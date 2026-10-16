import { createHash } from 'node:crypto'

import { expect, it, vi } from 'vitest'

import { runLocalAgent } from './agent'

it('runs a complete diagnostic tool loop inside Chromium without Node APIs', async () => {
    const network = vi
        .spyOn(globalThis, 'fetch')
        .mockRejectedValue(new Error('Unexpected browser network request'))
    let turns = 0
    const output = await runLocalAgent(
        {
            run_id: 'browser-test',
            provider_id: 'mock',
            model_id: 'mock',
            locale: 'en-US',
            context: 'Crash log snapshot',
        },
        {
            modelTurn: async (turn) => {
                expect(
                    turn.tools.map((tool) => (tool.function as { name: string }).name).sort(),
                ).toEqual(['grep_log_file', 'list_log_files', 'read_log_file'])
                return ++turns === 1
                    ? {
                          role: 'assistant',
                          content: '',
                          tool_calls: [{ id: 'call-browser', name: 'list_log_files', args: {} }],
                      }
                    : {
                          role: 'assistant',
                          content: 'A diagnostic result from the browser runtime.',
                      }
            },
            tool: async () => [{ id: 'log-1', name: 'latest.log', first_line: 1 }],
        },
        new AbortController().signal,
        () => {},
    )
    expect(turns).toBe(2)
    expect(output).toContain('diagnostic result')
    expect(network).not.toHaveBeenCalled()
    network.mockRestore()
})

it('supports the hashing dependency used by Deep Agents context offloading', () => {
    expect(createHash('sha256').update('abc').digest('hex')).toBe(
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
    )
})
