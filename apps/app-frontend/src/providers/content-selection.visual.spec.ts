import { expect, it, vi } from 'vitest'
import { ref } from 'vue'

import type { GameInstance } from '@/helpers/types'

import { createContentSelection } from './content-selection'
import type { ContentSelectionItem } from './content-selection-types'

vi.mock('@modrinth/ui', () => ({
    createContext: () => [vi.fn(), vi.fn()],
    defineMessages: (messages: unknown) => messages,
    useVIntl: () => ({
        formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
    }),
    usesTargetGameVersion: () => false,
}))
vi.mock('@/store/state', () => ({
    useTheming: () => ({ getFeatureFlag: () => true }),
}))
vi.mock('@/helpers/cache.js', () => ({
    get_project_many: vi.fn(),
    get_version_many: vi.fn(),
}))
vi.mock('@/helpers/content-identity', () => ({
    compareContentIdentities: vi.fn(() => null),
    contentIdentityFromInput: (input: unknown) => input,
    contentIdentityInputsFromSnapshot: vi.fn(() => []),
    resolveContentIdentities: vi.fn(() => []),
}))
vi.mock('@/helpers/curseforge', () => ({
    getCurseForgeFile: vi.fn(),
    getCurseForgeProjects: vi.fn(),
    previewCurseForgeFile: vi.fn(),
    queueCurseForgeFile: vi.fn(),
    queueCurseForgeWorld: vi.fn(),
}))
vi.mock('@/helpers/instance', () => ({
    get_content_snapshot: vi.fn().mockResolvedValue(null),
    list: vi.fn().mockResolvedValue([]),
    preview_project_with_dependencies: vi.fn(),
    queue_content_batch: vi.fn(),
    queue_project_with_dependencies: vi.fn(),
}))
vi.mock('@/helpers/resources', () => ({
    resourceProjectDocuments: vi.fn().mockResolvedValue([]),
    sourceResource: (provider: string, projectId: string) => ({
        kind: 'source',
        project: { source: provider, projectId },
    }),
}))
vi.mock('@/helpers/settings', () => ({
    getBrowseDefaultInstanceId: vi.fn(() => null),
    setBrowseDefaultInstanceId: vi.fn(),
}))

const instance = {
    id: 'instance-1',
    name: 'Example',
    loader: 'neoforge',
    game_version: '1.21.1',
} as GameInstance

it('routes pending-version cart items into the resource preview as automatic releases', async () => {
    const showInstall = vi.fn().mockResolvedValue({ job_id: 'job-1' })
    const selection = createContentSelection({
        addNotification: vi.fn(),
        handleError: vi.fn(),
        downloadManager: { jobs: ref([]) } as never,
    })
    selection.setPreviewModal({} as never)
    selection.setResourceInstallPreview({ showInstall } as never)
    selection.setTarget(instance)

    const item: ContentSelectionItem = {
        key: 'modrinth:pending',
        provider: 'modrinth',
        projectId: 'pending',
        providerProjectId: 'pending',
        contentType: 'mod',
        title: 'Pending mod',
        slug: 'pending',
    }
    await expect(selection.add(item)).resolves.toBe(true)

    await expect(selection.installSelected()).resolves.toBe(true)
    expect(showInstall).toHaveBeenCalledTimes(1)
    expect(showInstall).toHaveBeenCalledWith({
        instanceId: 'instance-1',
        instanceName: 'Example',
        selections: [
            {
                id: 'modrinth:pending',
                resource: {
                    kind: 'source',
                    project: { source: 'modrinth', projectId: 'pending' },
                },
                release: { kind: 'automatic' },
                resourceType: 'mod',
                scope: { kind: 'instance' },
            },
        ],
        installDependencies: true,
    })
    expect(selection.items.value.size).toBe(0)
    expect(selection.state.value).toBe('idle')
})
