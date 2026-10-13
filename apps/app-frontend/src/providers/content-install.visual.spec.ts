import { expect, it, vi } from 'vitest'

import { get_project, get_version_many } from '@/helpers/cache.js'
import { get_install_candidates } from '@/helpers/instance'

import { createContentInstall } from './content-install'

vi.mock('@modrinth/ui', () => ({
    createContext: () => [vi.fn(), vi.fn()],
    defineMessages: (messages: unknown) => messages,
    defineMessage: (message: unknown) => message,
    useDebugLogger: () => vi.fn(),
    useVIntl: () => ({
        formatMessage: (message: { defaultMessage: string }) => message.defaultMessage,
    }),
}))
vi.mock('@/store/state', () => ({ useTheming: () => ({ getFeatureFlag: () => false }) }))
vi.mock('@/helpers/analytics', () => ({ trackEvent: vi.fn() }))
vi.mock('@/helpers/events.js', () => ({ instance_listener: vi.fn().mockResolvedValue(() => {}) }))
vi.mock('@/helpers/cache.js', () => ({
    get_project: vi.fn(),
    get_version_many: vi.fn(),
    get_organization: vi.fn(),
    get_team: vi.fn(),
}))
vi.mock('@/helpers/instance', () => ({
    get_install_candidates: vi.fn(),
    get: vi.fn(),
    list: vi.fn(),
    edit: vi.fn(),
}))
vi.mock('@/helpers/instance-icons', () => ({
    getDisplayInstanceIcon: () => ({ url: '', frameless: false }),
}))
vi.mock('@/helpers/tags', () => ({ get_game_versions: vi.fn().mockResolvedValue([]) }))
vi.mock('@/helpers/install', () => ({
    install_create_instance: vi.fn(),
    install_create_modpack_instance: vi.fn(),
    installJobInstanceId: vi.fn(),
    wait_for_install_job: vi.fn(),
}))
vi.mock('./content-install-dep-notifications', () => ({
    createContentInstallDepNotifications: () => ({ notifyInstalledDependencies: vi.fn() }),
}))

it('closes the install target after queueing without reporting failure or settling the callback twice', async () => {
    vi.clearAllMocks()
    vi.mocked(get_project).mockResolvedValue({
        id: 'abcdefgh',
        title: 'Example',
        project_type: 'mod',
        versions: ['release'],
    })
    vi.mocked(get_version_many).mockResolvedValue([
        {
            id: 'release',
            date_published: '2026-01-01',
            game_versions: ['1.20.1'],
            loaders: ['fabric'],
        },
    ])
    vi.mocked(get_install_candidates).mockResolvedValue([
        {
            id: 'instance',
            name: 'Instance',
            game_version: '1.20.1',
            loader: 'fabric',
            icon_path: null,
            installed: false,
            compatible: true,
        },
    ] as Awaited<ReturnType<typeof get_install_candidates>>)
    const handleError = vi.fn()
    const push = vi.fn()
    const context = createContentInstall({
        router: { push } as unknown as Parameters<typeof createContentInstall>[0]['router'],
        handleError,
        addNotification: vi.fn(),
    })
    const callback = vi.fn()
    const hide = vi.fn(() => context.handleCancel())
    context.setContentInstallModal({ show: vi.fn(), hide })
    const showInstallSelection = vi
        .fn()
        .mockResolvedValue({ job: { job_id: 'job' }, activeRoots: ['modrinth:abcdefgh'] })
    context.setResourceInstallPreview({ showInstallSelection } as unknown as Parameters<
        typeof context.setResourceInstallPreview
    >[0])
    await context.install('abcdefgh', null, null, 'test', callback)
    await context.handleInstallToInstance(context.instances.value[0])
    expect(showInstallSelection).toHaveBeenCalledWith(
        expect.objectContaining({ installDependencies: false }),
    )
    expect(hide).toHaveBeenCalledOnce()
    expect(callback).toHaveBeenCalledExactlyOnceWith('release', ['abcdefgh'])
    expect(handleError).not.toHaveBeenCalled()
    context.handleNavigate(context.instances.value[0])
    expect(push).toHaveBeenCalledWith('/instance/instance')
    expect(callback).toHaveBeenCalledOnce()
    const navigationCallback = vi.fn()
    await context.install('abcdefgh', null, null, 'test', navigationCallback)
    context.handleNavigate(context.instances.value[0])
    expect(navigationCallback).toHaveBeenCalledExactlyOnceWith()
    context.handleCancel()
    expect(navigationCallback).toHaveBeenCalledOnce()
})
