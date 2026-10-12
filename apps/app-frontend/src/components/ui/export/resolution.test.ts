import assert from 'node:assert/strict'
import test from 'node:test'

import type { ExportResolutionSession } from '@/helpers/resources'

import { analysisReady, pendingFailures } from './resolution.ts'

const session = (): ExportResolutionSession => ({
    id: 'session',
    requestId: 'request',
    selectionRevision: 1,
    instanceRevision: 1,
    state: 'needsDecision',
    files: [
        {
            snapshot: {
                relativePath: 'a.jar',
                integrity: {
                    size: 4,
                    hashes: ['sha1', 'sha256', 'sha512'].map((algorithm) => ({
                        algorithm,
                        value: 'abc',
                    })),
                },
                revision: 1,
                identifiable: true,
                modifiedAt: null,
                categoryId: 'mods',
                categorySelected: true,
            },
            localError: null,
            resolutions: [
                { kind: 'noMatch', source: 'modrinth' },
                {
                    kind: 'failed',
                    source: 'curseforge',
                    code: 'sourceUnavailable',
                    message: 'offline',
                },
            ],
        },
    ],
})
test('request failures require explicit embedding and bind the choice to the exact snapshot', () => {
    const result = session()
    assert.equal(analysisReady(result, ['a.jar'], []), false)
    assert.equal(analysisReady(result, ['a.jar'], ['a.jar']), true)
    assert.deepEqual(pendingFailures(result, ['a.jar']), [
        { snapshot: result.files[0].snapshot, source: 'curseforge' },
    ])
    assert.equal(analysisReady(result, ['other.jar'], ['a.jar']), false)
})
test('unreadable files and incomplete analysis cannot be bypassed by embedding', () => {
    const result = session()
    result.files[0].localError = 'missing'
    assert.equal(analysisReady(result, ['a.jar'], ['a.jar']), false)
    result.files[0].localError = null
    result.state = 'refreshing'
    assert.equal(analysisReady(result, ['a.jar'], ['a.jar']), false)
})
test('successful no-match is ready without accepting a failure', () => {
    const result = session()
    result.state = 'ready'
    result.files[0].resolutions = [
        { kind: 'noMatch', source: 'modrinth' },
        { kind: 'noMatch', source: 'curseforge' },
    ]
    assert.equal(analysisReady(result, ['a.jar'], []), true)
})
