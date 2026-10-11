import { expect, it } from 'vitest'

import ProjectCard from '../components/project/card/ProjectCard.vue'
import { mountThemed } from './visual-harness'

for (const layout of ['list', 'compact', 'grid', 'gallery'] as const) {
    it(`shows both source labels without source action buttons in ${layout}`, async () => {
        const wrapper = await mountThemed(
            ProjectCard,
            {
                title: 'Shared resource',
                layout,
                provider: 'modrinth',
                providers: ['modrinth', 'curseforge', 'modrinth'],
            },
            'dark',
        )
        expect(wrapper.text().match(/Modrinth/g)).toHaveLength(1)
        expect(wrapper.text().match(/CurseForge/g)).toHaveLength(1)
        expect(wrapper.findAll('button').some((button) => button.text() === 'CurseForge')).toBe(
            false,
        )
        wrapper.unmount()
    })
}
