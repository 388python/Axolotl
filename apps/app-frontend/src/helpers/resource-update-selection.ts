import type { ContentItem } from '@modrinth/ui'

import type { InstallJobSnapshot } from './install'
import type { SourceReleaseRef } from './resources'

export interface ResourceEntryReview {
    instanceId: string
    instanceName: string
    entryIds: string[]
    intent: 'switchVersion' | 'update'
    release: SourceReleaseRef | null
    installDependencies: boolean
}

export async function reviewResourceEntryChanges(
    input: Omit<ResourceEntryReview, 'intent'>,
    show: (input: ResourceEntryReview) => Promise<InstallJobSnapshot | null>,
    track: (job: InstallJobSnapshot) => void,
): Promise<boolean> {
    const entryIds = [...new Set(input.entryIds)]
    if (!entryIds.length) return false
    const job = await show({
        ...input,
        entryIds,
        intent: input.release ? 'switchVersion' : 'update',
    })
    if (!job) return false
    track(job)
    return true
}

export function confirmedContentGroupKey(item: ContentItem): string | null {
    if (!item.instanceResourceId) return null
    const path = item.file_path?.replaceAll('\\', '/') ?? ''
    const world = path.match(/^saves\/[^/]+\/datapacks\//)?.[0]
    if (item.project_type === 'datapack' && !world) return null
    return JSON.stringify([item.instanceResourceId, item.project_type, world ?? 'instance'])
}

export function duplicateGroupUpdateEntries(items: ContentItem[], anchor: ContentItem): string[] {
    const group = confirmedContentGroupKey(anchor)
    if (!group) return []
    return [
        ...new Set(
            items
                .filter(
                    (item) =>
                        confirmedContentGroupKey(item) === group &&
                        item.instanceMaterializationState === 'present' &&
                        item.instanceCapabilities?.canUpdate !== false &&
                        (item.resourceUpdate != null || item.update != null),
                )
                .flatMap((item) => (item.instanceEntryId ? [item.instanceEntryId] : [])),
        ),
    ]
}
