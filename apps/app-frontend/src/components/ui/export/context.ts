import { FolderOpenIcon } from '@modrinth/assets'
import {
    commonMessages,
    createContext,
    injectNotificationManager,
    injectPopupNotificationManager,
    type MultiStageModal,
    useVIntl,
} from '@modrinth/ui'
import { save } from '@tauri-apps/plugin-dialog'
import { computed, nextTick, onBeforeUnmount, ref, type ShallowRef } from 'vue'
import type { ComponentExposed } from 'vue-component-type-helpers'

import { resourceOperationError } from '@/helpers/resource-operation-logic'
import {
    type ExportCategory,
    type ExportRefreshProgress,
    type ExportResolutionSession,
    resourceExportClose,
    resourceExportLegacyExecute,
    resourceExportRefresh,
    resourceExportRefreshRetry,
    type ResourceExportScan,
    resourceExportScan,
} from '@/helpers/resources'
import { highlightInFolder } from '@/helpers/utils'

import { categoryMessages, exportMessages } from './messages'
import { analysisReady, pendingFailures } from './resolution'
import {
    categoryLeaves,
    defaultCategories,
    type ExportSelectionDraft,
    reviseCategorySelection,
    reviseFileSelection,
    selectedFiles,
} from './selection'

export function createExportContext(
    modal: ShallowRef<ComponentExposed<typeof MultiStageModal> | null>,
    instance: () => { id: string; name: string },
) {
    const { formatMessage } = useVIntl()
    const { handleError } = injectNotificationManager()
    const notifications = injectPopupNotificationManager()
    const snapshot = ref<ResourceExportScan | null>(null)
    const scanning = ref(false)
    const error = ref('')
    const exporting = ref(false)
    const refreshing = ref(false)
    const resolution = ref<ExportResolutionSession | null>(null)
    const refreshProgress = ref<ExportRefreshProgress | null>(null)
    const refreshError = ref('')
    const embedded = ref<string[]>([])
    let refreshController: AbortController | undefined
    let refreshGeneration = 0
    let revision = 0
    const draft = ref<ExportSelectionDraft>({ categories: [], included: [], excluded: [] })
    const currentPath = ref('')
    const search = ref('')
    const selectedOnly = ref(false)
    const packageInfo = ref({ name: '', version: '1.0.0', description: '' })
    let controller: AbortController | undefined
    let generation = 0
    const files = computed(() => snapshot.value?.files ?? [])
    const selection = computed({
        get: () => selectedFiles(files.value, draft.value),
        set: (paths: string[]) => {
            draft.value = reviseFileSelection(files.value, draft.value, paths)
        },
    })
    const selectedSize = computed(() => {
        const paths = new Set(selection.value)
        const frozen = new Map(
            resolution.value?.files.map((file) => [
                file.snapshot.relativePath,
                file.snapshot.integrity.size,
            ]) ?? [],
        )
        return files.value.reduce(
            (size, file) =>
                size +
                (paths.has(file.relativePath)
                    ? (frozen.get(file.relativePath) ?? file.integrity.size ?? 0)
                    : 0),
            0,
        )
    })
    const canExport = computed(
        () =>
            !refreshing.value &&
            !refreshError.value &&
            analysisReady(resolution.value, selection.value, embedded.value),
    )
    const failedFiles = computed(
        () =>
            resolution.value?.files.filter(
                (file) => file.localError || file.resolutions.some((r) => r.kind === 'failed'),
            ) ?? [],
    )
    function cancelRefresh() {
        refreshGeneration++
        refreshController?.abort()
        refreshing.value = false
    }
    async function refresh(retry = false) {
        if (!snapshot.value || exporting.value) return
        cancelRefresh()
        refreshController = new AbortController()
        const active = refreshController
        const generation = refreshGeneration
        const paths = new Set(selection.value)
        const selected = files.value.filter((file) => paths.has(file.relativePath))
        const session = snapshot.value.sessionId
        const currentRevision = ++revision
        refreshing.value = true
        refreshError.value = ''
        refreshProgress.value = null
        embedded.value = []
        try {
            const onProgress = (progress: ExportRefreshProgress) => {
                if (
                    generation === refreshGeneration &&
                    progress.sessionId === session &&
                    progress.selectionRevision === currentRevision
                )
                    refreshProgress.value = progress
            }
            const failed =
                resolution.value?.files.flatMap((file) =>
                    file.resolutions
                        .filter((r) => r.kind === 'failed')
                        .map((r) => ({ snapshot: file.snapshot, source: r.source })),
                ) ?? []
            const result =
                retry && failed.length && !resolution.value?.files.some((file) => file.localError)
                    ? await resourceExportRefreshRetry(
                          session,
                          failed,
                          currentRevision,
                          onProgress,
                          active.signal,
                      )
                    : await resourceExportRefresh(
                          snapshot.value.instanceId,
                          session,
                          selected,
                          currentRevision,
                          onProgress,
                          active.signal,
                      )
            if (generation !== refreshGeneration) return
            resolution.value = result
        } catch (failure) {
            if (generation === refreshGeneration && !active.signal.aborted)
                refreshError.value = resourceOperationError(failure)
        } finally {
            if (generation === refreshGeneration) refreshing.value = false
        }
    }
    function embed(path: string, accepted: boolean) {
        embedded.value = accepted
            ? [...new Set([...embedded.value, path])]
            : embedded.value.filter((value) => value !== path)
    }
    function close() {
        cancelScan()
        cancelRefresh()
        if (snapshot.value) void resourceExportClose(snapshot.value.sessionId).catch(() => {})
        snapshot.value = null
        resolution.value = null
    }
    const categories = computed(
        () =>
            snapshot.value?.categories.filter(
                (category) => categoryFiles(category).length > 0 || category.children.length > 0,
            ) ?? [],
    )
    const treeItems = computed(() => [
        ...(snapshot.value?.directories.map((directory) => ({
            path: directory.relativePath,
            type: 'directory' as const,
            modified: directory.modifiedAt == null ? undefined : directory.modifiedAt / 1000,
        })) ?? []),
        ...files.value.map((file) => ({
            path: file.relativePath,
            type: 'file' as const,
            size: file.integrity.size ?? undefined,
            modified: file.modifiedAt == null ? undefined : file.modifiedAt / 1000,
        })),
    ])

    function categoryFiles(category: ExportCategory) {
        const ids = new Set(categoryLeaves(category).map((leaf) => leaf.id))
        return files.value.filter((file) => ids.has(file.categoryId))
    }
    function categoryState(category: ExportCategory) {
        const ids = categoryLeaves(category).map((leaf) => leaf.id)
        const selected = new Set(draft.value.categories)
        const count = ids.filter((id) => selected.has(id)).length
        return { checked: count === ids.length, indeterminate: count > 0 && count < ids.length }
    }
    function categoryTitle(category: ExportCategory) {
        return (
            category.title ??
            (categoryMessages[category.id as keyof typeof categoryMessages]
                ? formatMessage(categoryMessages[category.id as keyof typeof categoryMessages])
                : category.paths[0])
        )
    }
    function setCategory(category: ExportCategory, selected: boolean) {
        draft.value = reviseCategorySelection(draft.value, category, selected)
    }
    function restore() {
        draft.value = { ...draft.value, included: [], excluded: [] }
    }
    function cancelScan() {
        generation++
        controller?.abort()
        scanning.value = false
    }
    async function scan() {
        cancelScan()
        controller = new AbortController()
        const activeController = controller
        const request = generation
        scanning.value = true
        error.value = ''
        try {
            const result = await resourceExportScan(instance().id, activeController.signal)
            if (generation !== request) return
            snapshot.value = result
            draft.value = {
                categories: defaultCategories(result.categories),
                included: [],
                excluded: [],
            }
        } catch (failure) {
            if (generation === request && !activeController.signal.aborted)
                error.value = resourceOperationError(failure)
        } finally {
            if (generation === request) scanning.value = false
        }
    }
    function show() {
        close()
        snapshot.value = null
        draft.value = { categories: [], included: [], excluded: [] }
        currentPath.value = ''
        search.value = ''
        selectedOnly.value = false
        packageInfo.value = { name: instance().name, version: '1.0.0', description: '' }
        embedded.value = []
        revision = 0
        refreshError.value = ''
        refreshProgress.value = null
        modal.value?.setStage('categories')
        modal.value?.show()
        void scan()
    }
    async function exportPack() {
        if (exporting.value || !snapshot.value || !resolution.value || !canExport.value) return
        exporting.value = true
        try {
            const output = await save({
                defaultPath: `${packageInfo.value.name} ${packageInfo.value.version}.mrpack`,
                filters: [{ name: 'Modrinth Modpack', extensions: ['mrpack'] }],
            })
            if (!output) return
            await resourceExportLegacyExecute(
                snapshot.value.sessionId,
                resolution.value.selectionRevision,
                pendingFailures(resolution.value, embedded.value),
                {
                    name: packageInfo.value.name,
                    version: packageInfo.value.version,
                    summary: packageInfo.value.description,
                    author: null,
                },
                output,
            )
            notifications.addPopupNotification({
                title: formatMessage(exportMessages.complete),
                text: formatMessage(exportMessages.completeDescription, {
                    name: output.split(/[\\/]/).pop() ?? output,
                }),
                type: 'success',
                buttons: [
                    {
                        label: formatMessage(commonMessages.openInFolderButton),
                        icon: FolderOpenIcon,
                        action: () => highlightInFolder(output).catch(handleError),
                    },
                ],
            })
            exporting.value = false
            await nextTick()
            modal.value?.hide()
        } catch (failure) {
            refreshError.value = resourceOperationError(failure)
            resolution.value = null
            handleError(failure)
        } finally {
            exporting.value = false
        }
    }
    onBeforeUnmount(close)
    return {
        modal,
        snapshot,
        scanning,
        error,
        exporting,
        draft,
        currentPath,
        search,
        selectedOnly,
        packageInfo,
        files,
        categories,
        treeItems,
        selection,
        selectedSize,
        categoryFiles,
        categoryState,
        categoryTitle,
        setCategory,
        restore,
        scan,
        cancelScan,
        show,
        exportPack,
        formatMessage,
        refreshing,
        resolution,
        refreshProgress,
        refreshError,
        embedded,
        canExport,
        failedFiles,
        refresh,
        cancelRefresh,
        embed,
        close,
    }
}

export type ExportContext = ReturnType<typeof createExportContext>
export const [injectExportContext, provideExportContext] =
    createContext<ExportContext>('ExportModal')
