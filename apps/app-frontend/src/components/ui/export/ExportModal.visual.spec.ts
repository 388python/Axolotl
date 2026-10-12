import { Checkbox, FileTreeSelect, I18N_INJECTION_KEY, MultiStageModal } from '@modrinth/ui'
import { save } from '@tauri-apps/plugin-dialog'
import { flushPromises } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import {
    type ExportResolutionSession,
    resourceExportClose,
    resourceExportLegacyExecute,
    resourceExportRefresh,
    resourceExportRefreshRetry,
    type ResourceExportScan,
    resourceExportScan,
} from '@/helpers/resources'
import { mountThemed, waitFor } from '@/test/visual-harness'

import ExportModal from '../ExportModal.vue'

vi.mock('@/helpers/resources', () => ({
    resourceExportScan: vi.fn(),
    resourceExportRefresh: vi.fn(),
    resourceExportRefreshRetry: vi.fn(),
    resourceExportClose: vi.fn().mockResolvedValue(undefined),
    resourceExportLegacyExecute: vi.fn(),
}))
vi.mock('@/helpers/utils', () => ({ highlightInFolder: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(resourceExportClose).mockResolvedValue(undefined)
})

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

function resolved(revision = 1): ExportResolutionSession {
    return {
        id: 'fresh',
        requestId: 'request',
        selectionRevision: revision,
        instanceRevision: 1,
        state: 'ready',
        files: snapshot().files.map((file) => ({
            snapshot: {
                ...file,
                integrity: {
                    size: 10,
                    hashes: ['sha1', 'sha256', 'sha512'].map((algorithm) => ({
                        algorithm,
                        value: 'abc',
                    })),
                },
            },
            localError: null,
            resolutions: [
                { kind: 'noMatch', source: 'modrinth' },
                { kind: 'noMatch', source: 'curseforge' },
            ],
        })),
    }
}

it('cancels analysis on back navigation and ignores the previous revision after returning', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    let old!: (value: ExportResolutionSession) => void
    vi.mocked(resourceExportRefresh)
        .mockImplementationOnce(
            () =>
                new Promise((resolve) => {
                    old = resolve
                }),
        )
        .mockResolvedValueOnce(resolved(2))
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        const modal = wrapper.findComponent(MultiStageModal)
        modal.vm.setStage('package')
        await flushPromises()
        const signal = vi.mocked(resourceExportRefresh).mock.calls[0][5]!
        modal.vm.prevStage()
        await flushPromises()
        expect(signal.aborted).toBe(true)
        modal.vm.nextStage()
        await flushPromises()
        old({ ...resolved(), state: 'needsDecision', files: [] })
        await flushPromises()
        const button = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.trim() === 'Export')!
        expect(button.disabled).toBe(false)
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

it('retries exact failed references without silently choosing embedding', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    const result = resolved()
    result.state = 'needsDecision'
    result.files[0].resolutions[1] = {
        kind: 'failed',
        source: 'curseforge',
        code: 'offline',
        message: 'Offline',
    }
    vi.mocked(resourceExportRefresh).mockResolvedValue(result)
    vi.mocked(resourceExportRefreshRetry).mockResolvedValue(resolved(2))
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        wrapper.findComponent(MultiStageModal).vm.setStage('package')
        await flushPromises()
        const exportButton = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.trim() === 'Export')!
        expect(exportButton.disabled).toBe(true)
        const retry = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.includes('Retry analysis'))!
        retry.click()
        await flushPromises()
        expect(vi.mocked(resourceExportRefreshRetry).mock.calls[0][1]).toEqual([
            { snapshot: result.files[0].snapshot, source: 'curseforge' },
        ])
        expect(resourceExportRefresh).toHaveBeenCalledTimes(1)
        expect(exportButton.disabled).toBe(false)
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

it('shows serialized analysis errors and invalidates changed-file evidence before retry', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    vi.mocked(resourceExportRefresh)
        .mockRejectedValueOnce({ message: 'Source unavailable', code: 'resource' })
        .mockResolvedValueOnce(resolved())
    vi.mocked(save).mockResolvedValue('out.mrpack')
    vi.mocked(resourceExportLegacyExecute).mockRejectedValue({
        message: 'File changed after analysis',
        code: 'resource',
    })
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        wrapper.findComponent(MultiStageModal).vm.setStage('package')
        await flushPromises()
        expect(document.querySelector('[role="alert"]')?.textContent).toContain(
            'Source unavailable',
        )
        expect(document.body.textContent).not.toContain('[object Object]')
        const retry = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.includes('Retry analysis'))!
        retry.click()
        await flushPromises()
        const exportButton = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.trim() === 'Export')!
        expect(exportButton.disabled).toBe(false)
        exportButton.click()
        await flushPromises()
        expect(document.querySelector('[role="alert"]')?.textContent).toContain(
            'File changed after analysis',
        )
        expect(exportButton.disabled).toBe(true)
        expect(document.querySelector('[role="dialog"] input')).toBeTruthy()
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

it('locks explicit failure choices during export', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    const result = resolved()
    result.state = 'needsDecision'
    result.files[0].resolutions[1] = {
        kind: 'failed',
        source: 'curseforge',
        code: 'offline',
        message: 'Offline',
    }
    vi.mocked(resourceExportRefresh)
        .mockResolvedValueOnce(result)
        .mockImplementationOnce(() => new Promise(() => {}))
    vi.mocked(save).mockResolvedValue('out.mrpack')
    vi.mocked(resourceExportLegacyExecute).mockImplementationOnce(() => new Promise(() => {}))
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        const modal = wrapper.findComponent(MultiStageModal)
        modal.vm.setStage('package')
        await flushPromises()
        const checkbox = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="checkbox"]'),
        ].find((button) => button.textContent?.includes('Embed this file'))!
        checkbox.click()
        await flushPromises()
        const exportButton = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.trim() === 'Export')!
        expect(exportButton.disabled).toBe(false)
        exportButton.click()
        await flushPromises()
        expect(checkbox.disabled).toBe(true)
        const retry = [
            ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
        ].find((button) => button.textContent?.includes('Retry analysis'))!
        expect(retry.disabled).toBe(true)
        expect(resourceExportLegacyExecute).toHaveBeenCalledTimes(1)
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

it('refreshes on entry while package edits reuse evidence and returning triggers incremental refresh', async () => {
    vi.mocked(resourceExportScan).mockResolvedValue(snapshot())
    let complete!: (result: ExportResolutionSession) => void
    vi.mocked(resourceExportRefresh)
        .mockImplementationOnce(
            () =>
                new Promise((resolve) => {
                    complete = resolve
                }),
        )
        .mockResolvedValueOnce(resolved(2))
    const wrapper = await mountExport()
    try {
        wrapper.vm.show()
        await flushPromises()
        const modal = wrapper.findComponent(MultiStageModal)
        modal.vm.setStage('package')
        await flushPromises()
        expect(resourceExportRefresh).toHaveBeenCalledTimes(1)
        expect(document.querySelector('[role="status"]')?.textContent).toContain('Verifying')
        const inputs = document.querySelectorAll<HTMLInputElement>('[role="dialog"] input')
        inputs[0].value = 'Edited package'
        inputs[0].dispatchEvent(new Event('input', { bubbles: true }))
        await flushPromises()
        complete(resolved())
        await flushPromises()
        expect(resourceExportRefresh).toHaveBeenCalledTimes(1)
        modal.vm.prevStage()
        await flushPromises()
        modal.vm.nextStage()
        await flushPromises()
        expect(resourceExportRefresh).toHaveBeenCalledTimes(2)
        expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')?.value).toBe(
            'Edited package',
        )
        modal.vm.hide()
        await flushPromises()
        expect(resourceExportClose).toHaveBeenCalledWith('fresh')
    } finally {
        wrapper.unmount()
        vi.clearAllMocks()
    }
})

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
