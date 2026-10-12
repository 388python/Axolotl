<script setup lang="ts">
import { Button, Checkbox, commonMessages, StyledInput, useFormatBytes } from '@modrinth/ui'
import { onBeforeUnmount, onMounted } from 'vue'

import { injectExportContext } from './context'
import { exportMessages } from './messages'
const {
    packageInfo,
    exporting,
    selection,
    selectedSize,
    formatMessage,
    refreshing,
    refreshProgress,
    refreshError,
    resolution,
    failedFiles,
    embedded,
    refresh,
    cancelRefresh,
    embed,
} = injectExportContext()
onMounted(() => {
    void refresh()
})
onBeforeUnmount(cancelRefresh)
const formatBytes = useFormatBytes()
</script>
<template>
    <div class="flex min-w-0 flex-col gap-4">
        <div v-if="refreshing" class="flex flex-col gap-2" role="status">
            <span>{{ formatMessage(exportMessages.analyzing) }}</span>
            <progress
                v-if="refreshProgress"
                :max="Math.max(1, refreshProgress.total)"
                :value="refreshProgress.completed"
                class="w-full"
            />
        </div>
        <div v-else-if="refreshError" class="flex flex-col items-start gap-2" role="alert">
            <p class="m-0 break-all">{{ refreshError }}</p>
            <Button :disabled="exporting" @click="refresh(true)">{{
                formatMessage(exportMessages.retryAnalysis)
            }}</Button>
        </div>
        <template v-else-if="resolution">
            <p v-if="!failedFiles.length" class="m-0">
                {{ formatMessage(exportMessages.analyzed) }}
            </p>
            <div v-else class="flex flex-col gap-3">
                <p class="m-0">{{ formatMessage(exportMessages.failedAnalysis) }}</p>
                <div
                    v-for="file in failedFiles"
                    :key="file.snapshot.relativePath"
                    class="flex flex-col gap-2 rounded-xl border border-solid border-surface-4 p-3"
                >
                    <strong class="break-all">{{ file.snapshot.relativePath }}</strong>
                    <p v-if="file.localError" class="m-0 break-all">
                        {{ formatMessage(exportMessages.localFailure) }} {{ file.localError }}
                    </p>
                    <template v-else>
                        <p
                            v-for="failure in file.resolutions.filter((r) => r.kind === 'failed')"
                            :key="failure.source"
                            class="m-0 break-all"
                        >
                            {{ failure.source }}:
                            {{ failure.kind === 'failed' ? failure.message : '' }}
                        </p>
                        <Checkbox
                            :disabled="exporting"
                            :model-value="embedded.includes(file.snapshot.relativePath)"
                            :label="formatMessage(exportMessages.embedFailure)"
                            @update:model-value="embed(file.snapshot.relativePath, $event)"
                        />
                    </template>
                </div>
                <Button :disabled="exporting" class="self-start" @click="refresh(true)">{{
                    formatMessage(exportMessages.retryAnalysis)
                }}</Button>
            </div>
        </template>
        <p class="m-0">
            {{
                formatMessage(exportMessages.selection, {
                    count: selection.length,
                    size: formatBytes(selectedSize),
                })
            }}
        </p>
        <label class="flex flex-col gap-2">
            <span class="font-semibold">{{ formatMessage(exportMessages.name) }}</span>
            <StyledInput v-model="packageInfo.name" :disabled="exporting" wrapper-class="w-full" />
        </label>
        <label class="flex flex-col gap-2">
            <span class="font-semibold">{{ formatMessage(exportMessages.version) }}</span>
            <StyledInput
                v-model="packageInfo.version"
                :disabled="exporting"
                wrapper-class="w-full"
            />
        </label>
        <label class="flex flex-col gap-2">
            <span class="font-semibold">{{ formatMessage(commonMessages.descriptionLabel) }}</span>
            <StyledInput
                v-model="packageInfo.description"
                :disabled="exporting"
                multiline
                wrapper-class="w-full"
            />
        </label>
    </div>
</template>
