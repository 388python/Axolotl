import { flushPromises, mount } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import { useSearchResultTranslation } from './use-search-result-translation'

it('empty search snapshots cancel older translations without restoring old results or the wrong resource type', async () => {
    const projects = ref([{ title: 'Old project' }]),
        servers = ref([{ title: 'Old server' }])
    const projectType = ref('mod'),
        locale = ref('zh-CN')
    const pending: Array<(value: typeof projects.value) => void> = []
    const translate = vi.fn(
        () =>
            new Promise<typeof projects.value>((resolve) => {
                pending.push(resolve)
            }),
    )
    let control!: ReturnType<typeof useSearchResultTranslation>
    const wrapper = mount(
        defineComponent({
            setup: () => {
                control = useSearchResultTranslation({
                    projectHits: projects,
                    serverHits: servers,
                    projectType,
                    locale,
                    onError: vi.fn(),
                    translate: translate as never,
                })
                return () => h('div')
            },
        }),
    )
    try {
        projects.value = []
        await flushPromises()
        pending[0]([{ title: 'Stale translated project' }])
        await flushPromises()
        expect(projects.value).toEqual([])
        expect(control.translationActive.value).toBe(false)
        projectType.value = 'server'
        await flushPromises()
        projectType.value = 'mod'
        projects.value = [{ title: 'Current project' }]
        await flushPromises()
        pending[1]([{ title: 'Stale translated server' }])
        pending[2]([{ title: 'Translated current project' }])
        await flushPromises()
        expect(projects.value).toEqual([{ title: 'Translated current project' }])
        expect(servers.value).toEqual([{ title: 'Old server' }])
        expect(translate.mock.calls.at(-1)?.[0]).toEqual([{ title: 'Current project' }])
        locale.value = 'ja-JP'
        await flushPromises()
        expect(translate.mock.calls.at(-1)?.[0]).toEqual([{ title: 'Current project' }])
        pending[3]([{ title: 'Japanese current project' }])
        await flushPromises()
        control.toggle()
        expect(projects.value).toEqual([{ title: 'Current project' }])
    } finally {
        wrapper.unmount()
    }
})
