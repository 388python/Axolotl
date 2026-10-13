import { invoke } from '@tauri-apps/api/core'

import {
    resourceGet,
    resourceLocate,
    type ResourceType,
    type SourceProjectRef,
} from './resources.ts'

export async function resolveContentFavorite(favorite: ContentFavorite) {
    if (!favorite.resource_id) {
        return (await resourceLocate(favorite.provider, favorite.locator ?? favorite.project_id))
            .metadata
    }
    const resource = await resourceGet({ kind: 'resource', id: favorite.resource_id })
    const metadata = resource.projects.find(
        (project) =>
            project.project.source === resource.defaultProject.source &&
            project.project.projectId === resource.defaultProject.projectId,
    )
    if (!metadata) throw new Error('Favorite resource presentation is unavailable')
    return metadata
}

export type FavoriteProvider = 'modrinth' | 'curseforge' | 'mcarchive' | 'planet_minecraft'
export type FavoriteContentType = 'mod' | 'resourcepack' | 'datapack' | 'shader'

export interface ContentFavorite {
    locator?: string | null
    resource_id?: string | null
    projects?: SourceProjectRef[]
    provider: FavoriteProvider
    project_id: string
    content_type: FavoriteContentType
    saved_at: number
}

export interface ContentFavoriteInput {
    slug?: string | null
    provider: FavoriteProvider
    project_id: string
    content_type: FavoriteContentType
}

export const FAVORITE_CONTENT_TYPES: FavoriteContentType[] = [
    'mod',
    'resourcepack',
    'datapack',
    'shader',
]

export function isFavoriteContentType(value: string): value is FavoriteContentType {
    return FAVORITE_CONTENT_TYPES.includes(value as FavoriteContentType)
}

export function contentFavoriteKey(provider: FavoriteProvider, projectId: string) {
    return `${provider}:${projectId}`
}

export async function listContentFavorites(): Promise<ContentFavorite[]> {
    const values = await invoke<
        {
            resourceId: string | null
            locator: string | null
            project: SourceProjectRef
            projects: SourceProjectRef[]
            contentType: ResourceType
            savedAt: number
        }[]
    >('plugin:axolotl-resources|resource_favorites')
    return values.map((value) => ({
        resource_id: value.resourceId,
        locator: value.locator,
        projects: value.projects,
        provider: value.project.source,
        project_id: value.project.projectId,
        content_type: value.contentType.toLowerCase() as FavoriteContentType,
        saved_at: value.savedAt,
    }))
}

export async function addContentFavorite(favorite: ContentFavoriteInput): Promise<ContentFavorite> {
    const types: Record<FavoriteContentType, ResourceType> = {
        mod: 'mod',
        shader: 'shader',
        resourcepack: 'resourcePack',
        datapack: 'dataPack',
    }
    await invoke('plugin:axolotl-resources|resource_favorite_add', {
        project: { source: favorite.provider, projectId: favorite.project_id },
        contentType: types[favorite.content_type],
        locator: favorite.slug ?? null,
    })
    const values = await listContentFavorites()
    const saved = values.find((value) =>
        value.projects?.some(
            (project) =>
                project.source === favorite.provider && project.projectId === favorite.project_id,
        ),
    )
    if (!saved) throw new Error('Saved favorite could not be reloaded')
    return saved
}

export async function removeContentFavorite(
    provider: FavoriteProvider,
    projectId: string,
): Promise<void> {
    await invoke('plugin:axolotl-resources|resource_favorite_remove', {
        project: { source: provider, projectId },
    })
}
