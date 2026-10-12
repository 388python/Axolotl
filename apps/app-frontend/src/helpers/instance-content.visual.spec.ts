import { beforeEach, expect, it, vi } from 'vitest'

import { get_content_snapshot, type InstanceContentSnapshot } from './instance'
import { loadInstanceContentData, refreshInstanceContentUpdates } from './instance-content'
import { resourceCheckUpdates } from './resources'

vi.mock('./instance', () => ({ get_content_snapshot: vi.fn(), refresh_content: vi.fn() }))
vi.mock('./resources', () => ({ resourceCheckUpdates: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => path }))

beforeEach(() => vi.resetAllMocks())

const snapshot = (): InstanceContentSnapshot => ({
    instanceId: 'instance',
    revision: 1,
    pack: null,
    pendingManualDownloads: [],
    warnings: [],
    items: [
        {
            fileId: 'file',
            entryId: 'entry',
            memberId: null,
            ownershipKind: 'user_added',
            materializationState: 'present',
            overrideKind: 'none',
            expectedRelativePath: 'mods/example.jar',
            required: true,
            projectType: 'mod',
            provider: 'modrinth',
            providerProjectId: 'project',
            providerReleaseId: 'release',
            content: null,
            dependency: null,
            capabilities: {
                canToggle: true,
                canDelete: true,
                canUpdate: true,
                canChangeVersion: true,
                canRestorePackDefault: false,
            },
        },
    ],
})

it('returns local content without waiting for remote update discovery', async () => {
    vi.mocked(get_content_snapshot).mockResolvedValue(snapshot())
    vi.mocked(resourceCheckUpdates).mockImplementation(() => new Promise(() => {}))
    const data = await loadInstanceContentData('instance')
    expect(data?.contentItems[0].file_name).toBe('example.jar')
    expect(data?.contentItems[0].instanceCapabilities?.canUpdate).toBe(false)
    expect(resourceCheckUpdates).not.toHaveBeenCalled()
})

it('enriches updates separately and preserves the displayed snapshot', async () => {
    const original = snapshot()
    vi.mocked(get_content_snapshot).mockResolvedValue(original)
    vi.mocked(resourceCheckUpdates).mockResolvedValue({
        entries: ['entry'],
        releases: {
            entry: {
                project: { source: 'modrinth', projectId: 'project' },
                releaseId: 'new-release',
            },
        },
        unresolved: [],
        failures: [],
        failedEntries: [],
    })
    const data = (await loadInstanceContentData('instance'))!
    const enriched = await refreshInstanceContentUpdates(data)
    expect(enriched.contentItems[0].resourceUpdate?.releaseId).toBe('new-release')
    expect(enriched.contentItems[0].instanceCapabilities?.canUpdate).toBe(true)
    expect(data.contentItems[0].resourceUpdate).toBeNull()
    expect(data.contentItems[0].instanceCapabilities?.canUpdate).toBe(false)
    expect(original.items[0].capabilities.canUpdate).toBe(true)
})

it('keeps local content available when update discovery fails', async () => {
    vi.mocked(get_content_snapshot).mockResolvedValue(snapshot())
    vi.mocked(resourceCheckUpdates).mockRejectedValue(new Error('offline'))
    const data = (await loadInstanceContentData('instance'))!
    const enriched = await refreshInstanceContentUpdates(data)
    expect(enriched.contentItems).toHaveLength(1)
    expect(enriched.snapshot.warnings[0].code).toBe('resource_update_unavailable')
    expect(data.snapshot.warnings).toHaveLength(0)
})

it('discards update responses after the instance request is canceled', async () => {
    vi.mocked(get_content_snapshot).mockResolvedValue(snapshot())
    let resolve!: (result: Awaited<ReturnType<typeof resourceCheckUpdates>>) => void
    vi.mocked(resourceCheckUpdates).mockImplementation(
        () =>
            new Promise((done) => {
                resolve = done
            }),
    )
    const controller = new AbortController()
    const data = (await loadInstanceContentData('instance'))!
    const pending = refreshInstanceContentUpdates(data, controller.signal)
    controller.abort()
    resolve({
        entries: [],
        releases: {},
        unresolved: [],
        failures: ['late failure'],
        failedEntries: [],
    })
    expect(await pending).toBe(data)
    expect(resourceCheckUpdates).toHaveBeenCalledWith('instance', controller.signal)
})
