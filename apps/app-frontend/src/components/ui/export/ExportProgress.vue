<script setup lang="ts">
import { computed } from 'vue'

import { injectExportContext } from './context'
import { exportMessages } from './messages'

const {
    scanning,
    refreshing,
    refreshProgress,
    refreshError,
    resolution,
    failedFiles,
    formatMessage,
} = injectExportContext()
const active = computed(() => scanning.value || refreshing.value)
const label = computed(() => {
    if (scanning.value) return formatMessage(exportMessages.scan)
    if (active.value) {
        if (refreshProgress.value?.phase === 'complete')
            return formatMessage(exportMessages.finalizingAnalysis)
        if (refreshProgress.value?.phase === 'hashing')
            return formatMessage(exportMessages.verifyingLocal)
        if (refreshProgress.value)
            return formatMessage(exportMessages.verifyingSource, {
                source:
                    refreshProgress.value.phase === 'modrinth'
                        ? 'Modrinth'
                        : refreshProgress.value.phase === 'curseforge'
                          ? 'CurseForge'
                          : refreshProgress.value.phase === 'mcarchive'
                            ? 'MCArchive'
                            : refreshProgress.value.phase === 'planet_minecraft'
                              ? 'Planet Minecraft'
                              : refreshProgress.value.phase,
            })
        return formatMessage(exportMessages.analyzing)
    }
    if (refreshError.value || failedFiles.value.length)
        return formatMessage(exportMessages.checkNeedsAttention)
    if (resolution.value) return formatMessage(exportMessages.analyzed)
    return ''
})
</script>

<template>
    <div
        v-if="label"
        class="flex min-w-0 flex-col gap-1 text-sm font-normal"
        role="status"
        data-export-progress
    >
        <span>{{ label }}</span>
        <progress
            v-if="active"
            :max="Math.max(1, refreshProgress?.overallTotal ?? refreshProgress?.total ?? 1)"
            :value="
                !scanning && refreshProgress
                    ? (refreshProgress.overallCompleted ?? refreshProgress.completed) +
                      (refreshProgress.currentFileFraction ?? 0)
                    : undefined
            "
            class="h-1 w-full"
            :aria-label="label"
        />
    </div>
</template>
