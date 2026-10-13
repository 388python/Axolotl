import { expect, it, vi } from 'vitest'

import { get_project } from './cache.js'
import { resourceLocate } from './resources'

vi.mock('./resources', () => ({
    resourceCachePolicy: () => 'useFresh',
    resourceLocate: vi.fn(),
    resourceProjectDocuments: vi.fn(),
    resourceReleaseDocuments: vi.fn(),
    resourceRequest: vi.fn(),
}))

it('project links resolve slugs while retaining all source date semantics', async () => {
    vi.mocked(resourceLocate).mockResolvedValue({
        metadata: {
            createdAt: '2020-01-01',
            projectPublishedAt: '2021-01-01',
            updatedAt: '2025-01-01',
            firstPublishedAt: null,
            latestPublishedAt: null,
        },
        protocol: {
            id: 'abcdefgh',
            slug: 'sample-project',
            published: '2020-01-01',
            approved: '2021-01-01',
        },
    } as Awaited<ReturnType<typeof resourceLocate>>)
    const project = await get_project('sample-project')
    expect(resourceLocate).toHaveBeenCalledWith('modrinth', 'sample-project', undefined, 'useFresh')
    expect(project.id).toBe('abcdefgh')
    expect(project.resourceDates).toEqual({
        createdAt: '2020-01-01',
        projectPublishedAt: '2021-01-01',
        updatedAt: '2025-01-01',
        firstPublishedAt: null,
        latestPublishedAt: null,
    })
})
