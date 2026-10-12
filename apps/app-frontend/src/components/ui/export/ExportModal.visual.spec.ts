import { Checkbox, FileTreeSelect, I18N_INJECTION_KEY, MultiStageModal } from '@modrinth/ui'
import { flushPromises } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { type ResourceExportScan, resourceExportScan } from '@/helpers/resources'
import { mountThemed, waitFor } from '@/test/visual-harness'

import ExportModal from '../ExportModal.vue'

vi.mock('@/helpers/resources', () => ({ resourceExportScan: vi.fn() }))
vi.mock('@/helpers/instance', () => ({ export_instance_mrpack: vi.fn() }))
vi.mock('@/helpers/utils', () => ({ highlightInFolder: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))

const snapshot = (id = 'fresh'): ResourceExportScan => ({
    sessionId: id,
    instanceId: 'instance',
    instanceRevision: 1,
    categories: [
        {
            id: 'mods',
            defaultSelected: true,
            paths: ['mods/'],
            children: [],
            title: null,
            modifiedAt: null,
        },
    ],
    files: ['mods/a.jar', 'mods/deep/b.jar'].map((relativePath) => ({
        relativePath,
        integrity: { size: 10, hashes: [] },
        revision: 1,
        identifiable: true,
        modifiedAt: 1000,
        categoryId: 'mods',
        categorySelected: true,
    })),
    directories: [
        { relativePath: 'mods', modifiedAt: 1000 },
        { relativePath: 'mods/deep', modifiedAt: 1000 },
    ],
    excludedPaths: ['assets'],
    requirements: { gameVersion: '1.21.1', loader: 'neoforge', loaderVersion: '21.1.1' },
})
async function mountExport() {
    return mountThemed(ExportModal, { instance: { id: 'instance', name: 'Demo' } }, 'dark', {
        global: {
            directives: { tooltip: {} },
            provide: {
                [I18N_INJECTION_KEY as symbol]: {
                    locale: ref('en-US'),
                    t: (key: string) => key,
                    setLocale: () => {},
                },
                [Symbol.for('modrinth:notificationManager')]: { handleError: vi.fn() },
                [Symbol.for('modrinth:popupNotificationManager')]: {
                    addPopupNotification: vi.fn(),
                },
            },
        },
    })
}

it('keeps the exact selection and directory across category backtracking without rescanning', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        const modal = wrapper.findComponent(MultiStageModal)
        modal.vm.nextStage()
        await flushPromises()
        let tree = wrapper.findComponent(FileTreeSelect)
        expect(tree.props('modelValue')).toEqual(['mods/a.jar', 'mods/deep/b.jar'])
        tree.vm.$emit('update:modelValue', ['mods/a.jar'])
        tree.vm.$emit('update:currentPath', 'mods')
        await flushPromises()
        modal.vm.prevStage()
        await flushPromises()
        const mods = wrapper
            .findAllComponents(Checkbox)
            .find((checkbox) => checkbox.props('label') === 'Mods and plugins')!
        mods.vm.$emit('update:modelValue', false)
        await flushPromises()
        mods.vm.$emit('update:modelValue', true)
        modal.vm.nextStage()
        await flushPromises()
        tree = wrapper.findComponent(FileTreeSelect)
        expect(tree.props('modelValue')).toEqual(['mods/a.jar'])
        expect(tree.props('currentPath')).toBe('mods')
        expect(resourceExportScan).toHaveBeenCalledTimes(1)
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

it('cancels a hidden scan and ignores its response after reopening', async () => {
    let resolveOld!: (value: ResourceExportScan) => void
    vi.mocked(resourceExportScan)
        .mockImplementationOnce(
            () =>
                new Promise((resolve) => {
                    resolveOld = resolve
                }),
        )
        .mockResolvedValueOnce(snapshot())
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await waitFor(() => vi.mocked(resourceExportScan).mock.calls.length > 0)
        const signal = vi.mocked(resourceExportScan).mock.calls[0][1]!
        wrapper.findComponent(MultiStageModal).vm.hide()
        await flushPromises()
        expect(signal.aborted).toBe(true)
        wrapper.vm.show()
        await flushPromises()
        resolveOld({ ...snapshot('old'), files: [] })
        await flushPromises()
        wrapper.findComponent(MultiStageModal).vm.nextStage()
        await flushPromises()
        expect(
            wrapper
                .findComponent(FileTreeSelect)
                .props('items')
                .filter((item) => item.type === 'file'),
        ).toHaveLength(2)
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})
