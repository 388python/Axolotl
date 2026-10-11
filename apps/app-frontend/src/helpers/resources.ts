import { invoke } from '@tauri-apps/api/core'

import type { InstallJobSnapshot } from './install'

export type SourceId = 'modrinth' | 'curseforge' | 'mcarchive' | 'planet_minecraft'
export type ResourceType =
    'mod' | 'plugin' | 'dataPack' | 'resourcePack' | 'shader' | 'modPack' | 'world'
export interface SourceProjectRef {
    source: SourceId
    projectId: string
}
export interface SourceReleaseRef {
    project: SourceProjectRef
    releaseId: string
}
export type ResourceRef =
    { kind: 'resource'; id: string } | { kind: 'source'; project: SourceProjectRef }
export interface ResourceTarget {
    gameVersions: string[]
    loaders: string[]
    channels: string[]
}
export interface ResourceText {
    source: SourceId
    format: 'markdown' | 'html' | 'plain'
    body: string
}
export interface SourceProjectMetadata {
    environments: string[]
    project: SourceProjectRef
    slug: string | null
    title: string
    summary: string
    iconUrl: string | null
    types: ResourceType[]
    authors: string[]
    license: string | null
    downloads: number | null
    followers: number | null
    createdAt: string | null
    updatedAt: string | null
    firstPublishedAt: string | null
    latestPublishedAt: string | null
    categories: string[]
    gameVersions: string[]
    loaders: string[]
    gallery: string[]
}
export interface Resource {
    confirmedProjects: SourceProjectRef[]
    sourceFailures: { source: SourceId; code: string; message: string; retryable: boolean }[]
    id: string | null
    defaultProject: SourceProjectRef
    projects: SourceProjectMetadata[]
}
export interface ResourceArtifact {
    reference: { release: SourceReleaseRef; artifactId: string }
    fileName: string | null
    role: string
    available: boolean | null
    integrity: { size: number | null; hashes: { algorithm: string; value: string }[] }
    acquisition:
        | { kind: 'publicDownload'; urls: string[] }
        | { kind: 'manualDownload'; pageUrl: string }
        | { kind: 'unavailable' }
    dependencies: { target: unknown; kind: string; versionConstraint: string | null }[]
}
export interface ResourceRelease {
    reference: SourceReleaseRef
    version: string
    channel: string
    publishedAt: string | null
    compatibility: { gameVersions: string[]; loaders: string[] }
    artifacts: ResourceArtifact[]
    equivalentReleases: SourceReleaseRef[]
}
export interface ProjectDocument<T = unknown> {
    metadata: SourceProjectMetadata
    description: ResourceText
    protocol: T | null
}
export interface ReleaseDocument<T = unknown> {
    release: ResourceRelease
    changelog: ResourceText | null
    protocol: T | null
}
export interface ResourceSearchQuery {
    filter?: ResourceSearchExpression | null
    text: string
    types: ResourceType[]
    sources: SourceId[]
    target: ResourceTarget
    categories: string[]
    sort:
        | 'relevance'
        | 'downloads'
        | 'followers'
        | 'created'
        | 'projectUpdated'
        | 'firstPublished'
        | 'latestPublished'
    pageSize: number
}

export type ResourceSearchExpression =
    | { kind: 'all' | 'any'; terms: ResourceSearchExpression[] }
    | { kind: 'predicate'; field: string; values: string[]; excluded: boolean }
export interface ResourceSearchResult {
    entry: {
        queryEntryId: string
        resourceId: string | null
        defaultProject: SourceProjectRef
        candidates: SourceProjectRef[]
        status: 'confirmed' | 'unconfirmed' | 'rejected'
        basis: string[]
    }
    projects: SourceProjectMetadata[]
}
export interface ResourcePage<T> {
    requestId: string
    entries: T[]
    cursor: string | null
    hasMore: boolean
    total: { value: number | null; precision: 'exact' | 'estimate' | 'unknown' }
    sourceFailures: { source: SourceId; code: string; message: string; retryable: boolean }[]
    exhaustedSources: SourceId[]
}
export const emptyResourceTarget = (): ResourceTarget => ({
    gameVersions: [],
    loaders: [],
    channels: [],
})
export const sourceResource = (source: SourceId, projectId: string): ResourceRef => ({
    kind: 'source',
    project: { source, projectId },
})

export async function resourceRequest<T>(
    command: string,
    args: Record<string, unknown>,
    signal?: AbortSignal,
): Promise<T> {
    signal?.throwIfAborted()
    const requestId = crypto.randomUUID()
    const cancel = () => {
        void invoke('plugin:axolotl-resources|resource_cancel', { requestId }).catch(() => {})
    }
    signal?.addEventListener('abort', cancel, { once: true })
    try {
        const result = await invoke<T>(`plugin:axolotl-resources|${command}`, {
            ...args,
            requestId,
        })
        signal?.throwIfAborted()
        return result
    } finally {
        signal?.removeEventListener('abort', cancel)
    }
}
export function resourceSearch(
    query: ResourceSearchQuery,
    cursor: string | null,
    signal?: AbortSignal,
) {
    return resourceRequest<ResourcePage<ResourceSearchResult>>(
        'resource_search',
        { query, cursor },
        signal,
    )
}
export function resourceGet(
    reference: ResourceRef,
    source?: SourceId,
    locator?: string,
    signal?: AbortSignal,
) {
    return resourceRequest<Resource>(
        'resource_get',
        { reference, source: source ?? null, locator: locator ?? null },
        signal,
    )
}
export function resourceDescription(
    reference: ResourceRef,
    source: SourceId,
    locator?: string,
    signal?: AbortSignal,
) {
    return resourceRequest<ResourceText>(
        'resource_get_description',
        { reference, source, locator: locator ?? null },
        signal,
    )
}
export function resourceLocate<T>(
    source: SourceId,
    locator: string,
    signal?: AbortSignal,
    cachePolicy: ResourceCachePolicy = 'useFresh',
) {
    return resourceRequest<ProjectDocument<T>>(
        'resource_locate',
        { source, locator, cachePolicy },
        signal,
    )
}
export async function resourceProjectProtocol<T>(
    source: SourceId,
    locator: string,
    signal?: AbortSignal,
    cachePolicy: ResourceCachePolicy = 'useFresh',
): Promise<T> {
    const document = await resourceLocate<T>(source, locator, signal, cachePolicy)
    if (!document.protocol) throw new Error('Source presentation is unavailable')
    return document.protocol
}
export function resourceReleaseDocuments<T>(
    source: SourceId,
    releaseIds: string[],
    projectId?: string,
    signal?: AbortSignal,
    cachePolicy: ResourceCachePolicy = 'useFresh',
) {
    return resourceRequest<ReleaseDocument<T>[]>(
        'resource_release_documents',
        { source, releaseIds, projectId: projectId ?? null, cachePolicy },
        signal,
    )
}

export function resourceProjectDocuments<T>(
    source: SourceId,
    projectIds: string[],
    signal?: AbortSignal,
    cachePolicy: ResourceCachePolicy = 'useFresh',
) {
    return resourceRequest<ProjectDocument<T>[]>(
        'resource_project_documents',
        { source, projectIds, cachePolicy },
        signal,
    )
}
export function resourceReleases(
    reference: ResourceRef,
    target = emptyResourceTarget(),
    cursor: string | null = null,
    locator?: string,
    signal?: AbortSignal,
) {
    return resourceRequest<ResourcePage<ResourceRelease>>(
        'resource_list_releases',
        { reference, target, cursor, locator: locator ?? null },
        signal,
    )
}
export async function resourceChangelog(reference: SourceReleaseRef, signal?: AbortSignal) {
    const result = await resourceRequest<{
        release: SourceReleaseRef
        publishedAt: string | null
        text: ResourceText
    }>('resource_get_changelog', { releaseRef: reference, locator: null }, signal)
    return result.text
}
export function resourcePreferredSource() {
    return invoke<SourceId>('plugin:axolotl-resources|resource_preferred_source')
}
export function resourceSetPreferredSource(source: SourceId) {
    return invoke<void>('plugin:axolotl-resources|resource_set_preferred_source', { source })
}

export type ResourceCachePolicy = 'useFresh' | 'forceRefresh' | 'onlyCached'

export type ResourceOperationIntent = 'install' | 'update' | 'switchVersion'
export type ResourceSelection = {
    id: string
    resource: ResourceRef
    release:
        | { kind: 'automatic' }
        | { kind: 'release'; reference: SourceReleaseRef }
        | { kind: 'artifact'; reference: ResourceArtifact['reference'] }
    resourceType: ResourceType
    scope: { kind: 'instance' } | { kind: 'world'; relativePath: string }
    entryId?: string | null
}
export type InstalledResource = {
    entryId: string
    instanceId: string
    resourceId: string | null
    artifact: ResourceArtifact['reference'] | null
    knownArtifacts: ResourceArtifact['reference'][]
    integrity: ResourceArtifact['integrity']
    relativePath: string
    originalFileName: string | null
    resourceType: ResourceType
    scope: ResourceSelection['scope']
    enabled: boolean
    missing: boolean
    ownership: { kind: 'userAdded' | 'packManaged' | 'localDiscovered'; packMemberId?: string }
}
export type ResourceDependencyNode = {
    id: string
    resource: ResourceRef
    scope: ResourceSelection['scope']
    state: string
    artifact: ResourceArtifact | null
    installedEntries: string[]
    compatibility: {
        gameVersion: 'compatible' | 'incompatible' | 'unknown'
        loader: 'compatible' | 'incompatible' | 'unknown'
        reasons: string[]
    }
}
export function resourceListInstalled(instanceId: string, signal?: AbortSignal) {
    return resourceRequest<InstalledResource[]>('resource_list_installed', { instanceId }, signal)
}
export function resourceCheckUpdates(instanceId: string, signal?: AbortSignal) {
    return resourceRequest<{
        entries: string[]
        releases: Record<string, SourceReleaseRef>
        unresolved: string[]
        failures: string[]
        failedEntries: string[]
    }>('resource_check_updates', { instanceId }, signal)
}
export function resourcePrepareEntryOperation(
    instanceId: string,
    intent: ResourceOperationIntent,
    entryIds: string[],
    release: SourceReleaseRef | null,
    installDependencies: boolean,
    signal?: AbortSignal,
) {
    return resourceRequest<ResourceOperationPlan>(
        'resource_prepare_entry_operation',
        { instanceId, intent, entryIds, release, installDependencies },
        signal,
    )
}
export type ResourceOperationPlan = {
    schemaVersion: number
    id: string
    revision: number
    instanceId: string
    instanceRevision: number
    intent: ResourceOperationIntent
    selections: ResourceSelection[]
    activeRoots: string[]
    nodes: ResourceDependencyNode[]
    edges: {
        parent: string
        child: string
        kind: string
        declaredBy: ResourceArtifact['reference']
        versionConstraint: string | null
    }[]
    actions: {
        entryId: string
        nodeId: string
        kind: string
        targetRelativePath: string
        current: InstalledResource | null
        target: ResourceArtifact
    }[]
    issues: { blocking: boolean; code: string; message: string; selectionId: string | null }[]
    exceptions: {
        selectionId: string
        release: SourceReleaseRef
        dimensions: ('gameVersion' | 'loader')[]
    }[]
    selectionReasons: unknown[]
}
export type ResourcePlanDecision =
    | { kind: 'addOptional'; nodeId: string }
    | { kind: 'removeOptional'; nodeId: string }
    | { kind: 'removeRoot'; selectionId: string }
    | { kind: 'acceptCompatibility'; exception: unknown }
    | { kind: 'selectRelease'; nodeId: string; selection: ResourceSelection['release'] }

export function resourcePrepareOperation(
    instanceId: string,
    intent: ResourceOperationIntent,
    selections: ResourceSelection[],
    installDependencies = true,
    signal?: AbortSignal,
) {
    return resourceRequest<ResourceOperationPlan>(
        'resource_prepare_operation',
        {
            instanceId,
            intent,
            selections,
            installDependencies,
        },
        signal,
    )
}
export function resourceRevisePlan(
    planId: string,
    revision: number,
    decisions: ResourcePlanDecision[],
    signal?: AbortSignal,
) {
    return resourceRequest<ResourceOperationPlan>(
        'resource_revise_plan',
        { planId, revision, decisions },
        signal,
    )
}
export function resourceQueuePlan(planId: string, revision: number) {
    return invoke<InstallJobSnapshot>('plugin:axolotl-resources|resource_queue_plan', {
        planId,
        revision,
    })
}

export type ResourceHistoryEntry = {
    id: string
    entryId: string
    instanceId: string
    recordedAt: string
    previous: InstalledResource
    target: InstalledResource
    backupRelativePath: string | null
    backupAvailable: boolean
}

export function resourceListHistory(instanceId: string, entryId?: string | null) {
    return invoke<ResourceHistoryEntry[]>('plugin:axolotl-resources|resource_list_history', {
        instanceId,
        entryId: entryId ?? null,
    })
}

export function resourceRollback(instanceId: string, entryId: string, historyId: string) {
    return invoke<InstallJobSnapshot>('plugin:axolotl-resources|resource_rollback', {
        instanceId,
        entryId,
        historyId,
    })
}
export function resourceCachePolicy(behavior?: string): ResourceCachePolicy {
    return behavior === 'bypass'
        ? 'forceRefresh'
        : behavior === 'cache_only'
          ? 'onlyCached'
          : 'useFresh'
}
