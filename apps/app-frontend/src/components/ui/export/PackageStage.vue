<script setup lang="ts">
import { commonMessages, StyledInput, useFormatBytes } from '@modrinth/ui'

import { injectExportContext } from './context'
import { exportMessages } from './messages'
const { packageInfo, exporting, selection, selectedSize, formatMessage } = injectExportContext()
const formatBytes = useFormatBytes()
</script>
<template>
    <div class="flex min-w-0 flex-col gap-4">
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
