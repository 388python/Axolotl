import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { InstallJobSnapshot } from '../../../helpers/install'
import { observeExportTask } from './task.ts'

test('export observation preserves an active task through query failure and reports real cancellation', async () => {
    let queries = 0
    const result = await observeExportTask(
        async () => {
            queries++
            if (queries === 1) throw new Error('offline')
            return { status: queries === 2 ? 'running' : 'canceled' } as InstallJobSnapshot
        },
        new AbortController().signal,
        async () => {},
    )
    assert.equal(queries, 3)
    assert.equal(result?.status, 'canceled')
})

test('disposing the export observer does not reinterpret a running task as failed', async () => {
    const controller = new AbortController()
    const result = await observeExportTask(
        async () => ({ status: 'running' }) as InstallJobSnapshot,
        controller.signal,
        async () => {
            controller.abort()
        },
    )
    assert.equal(result, null)
})
