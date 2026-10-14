import { expect, it, vi } from 'vitest'

import { get_project, get_version_many } from '@/helpers/cache.js'
import { getCurseForgeFile, getCurseForgeFiles, getCurseForgeProject } from '@/helpers/curseforge'
import { get, get_install_candidates } from '@/helpers/instance'

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
vi.mock('@/helpers/curseforge', () => ({
    getCurseForgeFile: vi.fn(),
    getCurseForgeFiles: vi.fn(),
    getCurseForgeProject: vi.fn(),
    getCurseForgeImageUrl: (url: string | undefined) => url,
    getCurseForgeDownloadFailureDetails: () => null,
    installCurseForgeModpack: vi.fn(),
    queueCurseForgeWorld: vi.fn(),
    summarizeCurseForgeInstall: vi.fn(),
}))
vi.mock('./content-install-dep-notifications', () => ({
    createContentInstallDepNotifications: () => ({ notifyInstalledDependencies: vi.fn() }),
}))

function installationContext() {
    const context = createContentInstall({
        router: { push: vi.fn() } as never,
        handleError: vi.fn(),
        addNotification: vi.fn(),
    })
    const modal = { show: vi.fn(), hide: vi.fn(() => context.handleCancel()) }
    context.setContentInstallModal(modal)
    return { context, modal }
}

function project(id: string) {
    return { id, title: id, project_type: 'mod', versions: [`${id}-release`] } as never
}

function release(id: string) {
    return {
        id: `${id}-release`,
        date_published: '2026-01-01',
        game_versions: ['1.20.1'],
        loaders: ['fabric'],
    } as never
}

it('version loading failure closes and settles its own session once', async () => {
    vi.clearAllMocks()
    vi.mocked(get_project).mockResolvedValue(project('failed'))
    vi.mocked(get_version_many).mockRejectedValueOnce(new Error('version request failed'))
    const { context, modal } = installationContext()
    const callback = vi.fn()
    await expect(context.install('failed', null, null, 'test', callback)).rejects.toThrow(
        'version request failed',
    )
    expect(context.loading.value).toBe(false)
    expect(modal.hide).toHaveBeenCalled()
    expect(callback).toHaveBeenCalledOnce()
    context.handleCancel()
    expect(callback).toHaveBeenCalledOnce()
})

it('late project versions cannot overwrite a newer install session', async () => {
    vi.clearAllMocks()
    let finish!: (versions: never[]) => void
    vi.mocked(get_project).mockImplementation(async (id) => project(id))
    vi.mocked(get_version_many).mockImplementation((ids) =>
        ids[0] === 'old-release'
            ? new Promise((resolve) => {
                  finish = resolve
              })
            : Promise.resolve([release('new')]),
    )
    vi.mocked(get_install_candidates).mockImplementation(
        async (id) =>
            [
                {
                    id,
                    name: id,
                    game_version: '1.20.1',
                    loader: 'fabric',
                    installed: false,
                    compatible: true,
                },
            ] as never,
    )
    const { context } = installationContext()
    const oldCallback = vi.fn()
    const newCallback = vi.fn()
    const pending = context.install('old', null, null, 'test', oldCallback)
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'))
    await context.install('new', null, null, 'test', newCallback)
    finish([release('old')])
    await pending
    expect(context.instances.value[0].name).toBe('new')
    expect(context.loading.value).toBe(false)
    expect(oldCallback).toHaveBeenCalledOnce()
    expect(newCallback).not.toHaveBeenCalled()
})

it('cancelled version requests cannot repopulate the install target', async () => {
    vi.clearAllMocks()
    let finish!: (versions: never[]) => void
    vi.mocked(get_project).mockResolvedValue(project('cancelled'))
    vi.mocked(get_version_many).mockImplementationOnce(
        () =>
            new Promise((resolve) => {
                finish = resolve
            }),
    )
    const { context, modal } = installationContext()
    const callback = vi.fn()
    const pending = context.install('cancelled', null, null, 'test', callback)
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'))
    const shows = modal.show.mock.calls.length
    context.handleCancel()
    finish([release('cancelled')])
    await pending
    expect(context.instances.value).toEqual([])
    expect(context.loading.value).toBe(false)
    expect(modal.show).toHaveBeenCalledTimes(shows)
    expect(callback).toHaveBeenCalledOnce()
})

it('fixed CurseForge files are read by ID without loading the latest file page', async () => {
    vi.clearAllMocks()
    vi.mocked(getCurseForgeProject).mockResolvedValue({
        id: 42,
        name: 'CF project',
        classId: 6,
        slug: 'cf-project',
        authors: [],
    } as never)
    vi.mocked(getCurseForgeFile).mockResolvedValue({
        id: 7,
        displayName: 'Old release',
        fileName: 'old.jar',
        fileDate: '2025-01-01',
        gameVersions: ['1.20.1', 'Fabric'],
        isAvailable: true,
        hashes: [],
    } as never)
    vi.mocked(get_install_candidates).mockResolvedValue([])
    const { context } = installationContext()
    await context.installCurseForge('42', '7')
    expect(getCurseForgeFile).toHaveBeenCalledExactlyOnceWith(42, 7)
    expect(getCurseForgeFiles).not.toHaveBeenCalled()
    context.handleCancel()
})

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

it('direct instance installation settles the caller when resource review is cancelled or the instance vanished', async () => {
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
    ] as never)
    vi.mocked(get).mockResolvedValue({
        id: 'instance',
        name: 'Target',
        loader: 'fabric',
        game_version: '1.20.1',
    } as never)
    const context = createContentInstall({
        router: { push: vi.fn() } as never,
        handleError: vi.fn(),
        addNotification: vi.fn(),
    })
    context.setResourceInstallPreview({
        showInstallSelection: vi.fn().mockResolvedValue(null),
    } as never)
    const callback = vi.fn()
    await context.install('abcdefgh', 'release', 'instance', 'test', callback)
    expect(callback).toHaveBeenCalledExactlyOnceWith()
    vi.mocked(get).mockResolvedValue(null)
    const missing = vi.fn()
    await context.install('abcdefgh', 'release', 'missing', 'test', missing)
    expect(missing).toHaveBeenCalledExactlyOnceWith()
})
