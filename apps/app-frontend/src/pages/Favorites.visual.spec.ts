import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'

import Favorites from './Favorites.vue'

const fixture = vi.hoisted(() => ({
    load: vi.fn(),
    metadata: vi.fn(),
    refresh: null as null | (() => void),
}))
vi.mock('vue-router', () => ({
    useRoute: () => ({ query: {}, fullPath: '/browse/favorites', path: '/browse/favorites' }),
    useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
    onBeforeRouteLeave: () => {},
}))
vi.mock('@/store/breadcrumbs', () => ({
    useBreadcrumbs: () => ({ setName: vi.fn(), setContext: vi.fn() }),
}))
vi.mock('@/store/navigation-return', () => ({
    isBrowseReturnSourcePath: () => false,
    useNavigationReturnStore: () => ({ consumeBrowseReturnSnapshot: () => null }),
}))
vi.mock('@/helpers/page-transition', () => ({
    runAfterPageTransitionSettle: (run: () => void) => {
        fixture.refresh = run
        run()
    },
}))
vi.mock('@/helpers/cache.js', () => ({ get_project: vi.fn(), get_version_many: vi.fn() }))
vi.mock('@/helpers/curseforge', () => ({ getCurseForgeFiles: vi.fn() }))
vi.mock('@/helpers/settings', () => ({
    getLastBrowseContentDisplayMode: () => 'list',
    setLastBrowseContentDisplayMode: vi.fn(),
}))
vi.mock('@/helpers/content-favorites', () => ({
    contentFavoriteKey: (provider: string, id: string) => `${provider}:${id}`,
    prefetchContentFavorites: async () => new Map(),
    resolveContentFavorite: fixture.metadata,
}))
vi.mock('@/composables/useContentFavorites', async () => {
    const { ref } = await import('vue')
    return {
        useContentFavorites: () => {
            const favorites = ref([])
            const loaded = ref(false)
            return {
                favorites,
                loaded,
                isPending: () => false,
                load: async () => {
                    const result = await fixture.load()
                    favorites.value = result
                    loaded.value = true
                    return result
                },
            }
        },
    }
})
vi.mock('@/providers/content-selection', async () => {
    const { ref } = await import('vue')
    return {
        makeContentSelectionKey: () => '',
        injectContentSelection: () => ({
            targetInstance: ref(null),
            instances: ref([]),
            state: ref('idle'),
            progress: ref(null),
            selectedProjects: ref([]),
            selectedCount: ref(0),
            refreshInstances: vi.fn(),
            refreshInstalledIdentities: vi.fn(),
            isInstalling: () => false,
            isSelected: () => false,
        }),
    }
})
vi.mock('@/components/browse/BrowseInstanceSelector.vue', () => ({
    default: { render: () => null },
}))
vi.mock('@/components/ui/InstanceIcon.vue', () => ({ default: { render: () => null } }))
vi.mock('@modrinth/ui', async () => {
    const { defineComponent, h, ref } = await import('vue')
    const container = defineComponent({
        setup:
            (_, { slots }) =>
            () =>
                h('div', slots.default?.()),
    })
    return {
        ...Object.fromEntries(
            [
                'BrowseInstallHeader',
                'LoadingIndicator',
                'NavTabs',
                'Pagination',
                'PopoutMenu',
                'ProjectCardList',
                'SelectedProjectsFloatingBar',
                'StyledInput',
            ].map((name) => [name, container]),
        ),
        Button: defineComponent({
            setup:
                (_, { slots }) =>
                () =>
                    h('button', slots.default?.()),
        }),
        Admonition: defineComponent({
            props: ['header'],
            setup:
                (props, { slots }) =>
                () =>
                    h('aside', { role: 'alert' }, [props.header, slots.default?.()]),
        }),
        EmptyState: defineComponent({
            props: ['heading'],
            setup:
                (props, { slots }) =>
                () =>
                    h('section', [props.heading, slots.actions?.()]),
        }),
        ProjectCard: defineComponent({
            props: ['title'],
            setup: (props) => () => h('article', { 'data-favorite': '' }, props.title),
        }),
        getLatestMatchingInstallVersion: vi.fn(),
        getTargetInstallPreferences: vi.fn(),
        commonMessages: { retryButton: { defaultMessage: 'Retry' } },
        defineMessage: (value: unknown) => value,
        defineMessages: (value: unknown) => value,
        injectNotificationManager: () => ({ handleError: vi.fn() }),
        useStickyObserver: () => ({ isStuck: ref(false) }),
        useVIntl: () => ({
            formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
        }),
    }
})

const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => {
    wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
    vi.clearAllMocks()
})
function render() {
    const wrapper = mount(Favorites, { global: { directives: { tooltip: {} } } })
    wrappers.push(wrapper)
    return wrapper
}

it('shows a collection read failure with retry, then reports a genuine empty collection', async () => {
    fixture.load.mockRejectedValueOnce(new Error('read failed')).mockResolvedValue([])
    const wrapper = render()
    await vi.waitFor(() => expect(wrapper.text()).toContain('Failed to load favorites'))
    expect(wrapper.text()).not.toContain('No favorites yet')
    await wrapper
        .findAll('button')
        .find((button) => button.text() === 'Retry')!
        .trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('No favorites yet'))
    expect(wrapper.text()).not.toContain('Failed to load favorites')
})

it('preserves existing favorites after refresh failure and keeps item metadata failure local', async () => {
    fixture.load.mockResolvedValue([
        { provider: 'modrinth', project_id: 'one', content_type: 'mod', saved_at: 1 },
    ])
    fixture.metadata.mockRejectedValue(new Error('metadata unavailable'))
    const wrapper = render()
    await vi.waitFor(() => expect(wrapper.findAll('[data-favorite]')).toHaveLength(1))
    expect(wrapper.text()).not.toContain('Failed to load favorites')
    fixture.load.mockRejectedValue(new Error('refresh failed'))
    fixture.refresh!()
    await flushPromises()
    await vi.waitFor(() => expect(wrapper.find('[role="alert"]').exists()).toBe(true))
    expect(wrapper.findAll('[data-favorite]')).toHaveLength(1)
})
