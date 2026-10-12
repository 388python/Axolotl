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

import { export_instance_mrpack } from '@/helpers/instance'
import {
    type ExportCategory,
    type ResourceExportScan,
    resourceExportScan,
} from '@/helpers/resources'
import { highlightInFolder } from '@/helpers/utils'

import { categoryMessages, exportMessages } from './messages'
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
        return files.value.reduce(
            (size, file) => size + (paths.has(file.relativePath) ? (file.integrity.size ?? 0) : 0),
            0,
        )
    })
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
                error.value = String(failure)
        } finally {
            if (generation === request) scanning.value = false
        }
    }
    function show() {
        cancelScan()
        snapshot.value = null
        draft.value = { categories: [], included: [], excluded: [] }
        currentPath.value = ''
        search.value = ''
        selectedOnly.value = false
        packageInfo.value = { name: instance().name, version: '1.0.0', description: '' }
        modal.value?.setStage('categories')
        modal.value?.show()
        void scan()
    }
    async function exportPack() {
        if (exporting.value || !snapshot.value) return
        exporting.value = true
        try {
            const output = await save({
                defaultPath: `${packageInfo.value.name} ${packageInfo.value.version}.mrpack`,
                filters: [{ name: 'Modrinth Modpack', extensions: ['mrpack'] }],
            })
            if (!output) return
            await export_instance_mrpack(
                snapshot.value.instanceId,
                output,
                selection.value,
                packageInfo.value.version,
                packageInfo.value.description,
                packageInfo.value.name,
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
            handleError(failure)
        } finally {
            exporting.value = false
        }
    }
    onBeforeUnmount(cancelScan)
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
    }
}

export type ExportContext = ReturnType<typeof createExportContext>
export const [injectExportContext, provideExportContext] =
    createContext<ExportContext>('ExportModal')
