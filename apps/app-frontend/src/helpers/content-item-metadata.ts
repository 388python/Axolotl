import type { ContentItem } from '@modrinth/ui'

export function mergeContentItemMetadata(item: ContentItem, metadata: ContentItem): ContentItem {
    const merged = { ...item, ...metadata }
    if (item.instanceEntryId) {
        merged.update = null
        merged.resourceUpdate = item.resourceUpdate ?? null
        merged.rollback = item.rollback
    }
    if (item.instanceCapabilities) {
        merged.instanceCapabilities = {
            ...item.instanceCapabilities,
            canUpdate:
                (merged.resourceUpdate != null || merged.update != null) &&
                merged.instanceMaterializationState === 'present',
        }
    }
    return merged
}
