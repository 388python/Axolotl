<script setup lang="ts">
import { ArrowLeftIcon, ChevronRightIcon } from '@modrinth/assets'
import { Button, useVIntl } from '@modrinth/ui'
import { computed, defineAsyncComponent } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import BackupRepositorySettings from './BackupRepositorySettings.vue'
import ResourceManagementSettings from './ResourceManagementSettings.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { storageMessages } from './storage/storageMessages'

const StorageSettings = defineAsyncComponent(() => import('./StorageSettings.vue'))
const route = useRoute()
const router = useRouter()
const { formatMessage } = useVIntl()
const analyzing = computed(() => route.query.page === 'storage-analysis')

function openAnalysis() {
    return router.push({
        query: { ...route.query, page: 'storage-analysis' },
        hash: '#storage-backups',
    })
}

function back() {
    const { page: _page, ...query } = route.query
    return router.push({ query, hash: '#storage-backups' })
}
</script>

<template>
    <div v-if="analyzing" class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center gap-3">
            <Button @click="back"
                ><ArrowLeftIcon />{{ formatMessage(storageMessages.backToStorage) }}</Button
            >
            <h2 class="m-0 text-lg font-semibold">
                {{ formatMessage(storageMessages.analysisTitle) }}
            </h2>
        </div>
        <StorageSettings />
    </div>
    <div v-else class="flex flex-col gap-6">
        <ResourceManagementSettings scope="storage-backups" />
        <BackupRepositorySettings />
        <SettingsSection>
            <SettingsRow id="settings-target-storage-analysis">
                <template #label>{{ formatMessage(storageMessages.analysisTitle) }}</template>
                <template #description>{{
                    formatMessage(storageMessages.analysisDescription)
                }}</template>
                <template #control>
                    <Button @click="openAnalysis"
                        >{{ formatMessage(storageMessages.openAnalysis) }}<ChevronRightIcon
                    /></Button>
                </template>
            </SettingsRow>
        </SettingsSection>
    </div>
</template>
