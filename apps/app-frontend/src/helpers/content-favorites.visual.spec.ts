import { expect, it, vi } from 'vitest'

import { resolveContentFavorite } from './content-favorites'
import { resourceGet, resourceLocate } from './resources'

vi.mock('./resources', () => ({ resourceGet: vi.fn(), resourceLocate: vi.fn() }))

it('uses the available confirmed source while preserving the saved favorite identity', async () => {
    vi.clearAllMocks()
    const metadata = {
        project: { source: 'curseforge', projectId: '123' },
        title: 'Available project',
    }
    vi.mocked(resourceGet).mockResolvedValue({
        defaultProject: metadata.project,
        projects: [metadata],
    } as Awaited<ReturnType<typeof resourceGet>>)
    const favorite = {
        provider: 'modrinth' as const,
        project_id: 'abcdefgh',
        resource_id: 'resource',
        content_type: 'mod' as const,
        saved_at: 10,
    }
    expect(await resolveContentFavorite(favorite)).toEqual(metadata)
    expect(resourceGet).toHaveBeenCalledExactlyOnceWith({ kind: 'resource', id: 'resource' })
    expect(resourceLocate).not.toHaveBeenCalled()
    expect(favorite.provider).toBe('modrinth')
})

it('retains a source-only historical locator and reports failure when the resource has no presentation', async () => {
    vi.clearAllMocks()
    const metadata = {
        project: { source: 'mcarchive', projectId: 'uuid' },
        title: 'Archive project',
    }
    vi.mocked(resourceLocate).mockResolvedValue({ metadata } as Awaited<
        ReturnType<typeof resourceLocate>
    >)
    const favorite = {
        provider: 'mcarchive' as const,
        project_id: 'uuid',
        locator: 'original-slug',
        content_type: 'mod' as const,
        saved_at: 10,
    }
    expect(await resolveContentFavorite(favorite)).toEqual(metadata)
    expect(resourceLocate).toHaveBeenCalledExactlyOnceWith('mcarchive', 'original-slug')
    vi.mocked(resourceGet).mockRejectedValue(new Error('all sources unavailable'))
    await expect(resolveContentFavorite({ ...favorite, resource_id: 'resource' })).rejects.toThrow(
        'all sources unavailable',
    )
})
