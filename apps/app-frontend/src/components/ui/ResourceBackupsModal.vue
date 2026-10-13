<script setup lang="ts">
import { Button, Checkbox, defineMessages, NewModal, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, ref } from 'vue'

import { resourceOperationError } from '@/helpers/resource-operation-logic'
import {
    type ResourceBackupFile,
    resourceExecuteHistoryCleanup,
    type ResourceHistoryCleanupPlan,
    resourcePrepareHistoryCleanup,
    resourceScanBackups,
} from '@/helpers/resources'

const emit = defineEmits<{ cleaned: [] }>()
const { formatMessage } = useVIntl()
const messages = defineMessages({
    title: { id: 'app.resources.backups.title', defaultMessage: 'Old files and rollback backups' },
    scan: { id: 'app.resources.backups.scan', defaultMessage: 'Scan again' },
    loading: { id: 'app.resources.backups.loading', defaultMessage: 'Checking old files…' },
    empty: {
        id: 'app.resources.backups.empty',
        defaultMessage: 'No old files or rollback backups found.',
    },
    valid: { id: 'app.resources.backups.valid', defaultMessage: 'Verified rollback backup' },
    orphan: {
        id: 'app.resources.backups.orphan',
        defaultMessage: 'Old file without an available rollback',
    },
    invalid: {
        id: 'app.resources.backups.invalid',
        defaultMessage: 'Recorded backup unavailable for rollback',
    },
    fileSize: { id: 'app.resources.backups.file-size', defaultMessage: '{size, number} bytes' },
    unknown: {
        id: 'app.resources.backups.unknown',
        defaultMessage: 'Unknown old file — cleanup unavailable',
    },
    missing: { id: 'app.resources.backups.missing', defaultMessage: 'File missing' },
    summary: {
        id: 'app.resources.backups.summary',
        defaultMessage: '{count, number} selected · {size, number} bytes',
    },
    review: { id: 'app.resources.backups.review', defaultMessage: 'Review selected cleanup' },
    warning: {
        id: 'app.resources.backups.warning',
        defaultMessage:
            'Deleting these files permanently removes {count, number} available rollback backups. Current installed files are kept.',
    },
    confirm: { id: 'app.resources.backups.confirm', defaultMessage: 'Delete these backups' },
    back: { id: 'app.resources.backups.back', defaultMessage: 'Back to selection' },
    close: { id: 'app.resources.backups.close', defaultMessage: 'Close' },
    removed: {
        id: 'app.resources.backups.removed',
        defaultMessage: '{count, number} files deleted.',
    },
    failed: {
        id: 'app.resources.backups.failed',
        defaultMessage:
            '{count, number} files could not be deleted. Scan again to check their state.',
    },
})
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const files = ref<ResourceBackupFile[]>([])
const selected = ref<string[]>([])
const plan = ref<ResourceHistoryCleanupPlan | null>(null)
const busy = ref(false)
const cleaning = ref(false)
const error = ref<string | null>(null)
const resultText = ref<string | null>(null)
const size = computed(() =>
    files.value
        .filter((file) => selected.value.includes(file.id))
        .reduce((sum, file) => sum + (file.integrity.size ?? 0), 0),
)
let instanceId = ''
let generation = 0
function hide() {
    generation++
}
function show(id: string) {
    generation++
    instanceId = id
    files.value = []
    selected.value = []
    plan.value = null
    resultText.value = null
    modal.value?.show()
    void scan()
}
async function scan() {
    const current = generation
    busy.value = true
    error.value = null
    plan.value = null
    try {
        const value = await resourceScanBackups(instanceId)
        if (current !== generation) return
        files.value = value
        selected.value = selected.value.filter((id) =>
            value.some((file) => file.id === id && file.exists && file.recognized),
        )
    } catch (failure) {
        if (current === generation) error.value = resourceOperationError(failure)
    } finally {
        if (current === generation) busy.value = false
    }
}
function select(id: string, checked: boolean) {
    selected.value = checked
        ? [...new Set([...selected.value, id])]
        : selected.value.filter((value) => value !== id)
    plan.value = null
}
async function review() {
    if (busy.value || !selected.value.length) return
    busy.value = true
    error.value = null
    const current = generation
    try {
        const value = await resourcePrepareHistoryCleanup(instanceId, [...selected.value])
        if (current === generation) plan.value = value
    } catch (failure) {
        if (current === generation) error.value = resourceOperationError(failure)
    } finally {
        if (current === generation) busy.value = false
    }
}
async function cleanup() {
    if (busy.value || !plan.value) return
    busy.value = true
    error.value = null
    const current = generation
    const reviewed = plan.value
    cleaning.value = true
    try {
        const result = await resourceExecuteHistoryCleanup(reviewed.id, reviewed.revision)
        if (current !== generation) return
        resultText.value = formatMessage(
            result.failures.length ? messages.failed : messages.removed,
            {
                count:
                    result.failures.length ||
                    reviewed.files.filter((file) => result.removed.includes(file.id)).length,
            },
        )
        if (result.failures.length)
            error.value = result.failures.map(([, failure]) => failure).join('\n')
        const affectedPaths = new Set(reviewed.files.map((file) => file.relativePath))
        const affected = new Set(
            files.value
                .filter((file) => affectedPaths.has(file.relativePath))
                .map((file) => file.id),
        )
        selected.value = selected.value.filter((id) => !affected.has(id))
        files.value = files.value.map((file) =>
            affected.has(file.id) ? { ...file, exists: false, rollbackAvailable: false } : file,
        )
        emit('cleaned')
    } catch (failure) {
        if (current === generation) error.value = resourceOperationError(failure)
    } finally {
        if (current === generation) {
            busy.value = false
            cleaning.value = false
            plan.value = null
        }
    }
}
onBeforeUnmount(hide)
defineExpose({ show })
</script>

<template>
    <NewModal
        ref="modal"
        :header="formatMessage(messages.title)"
        :on-hide="hide"
        :disable-close="cleaning"
        scrollable
        width="min(40rem, 90vw)"
    >
        <div class="flex flex-col gap-3">
            <p v-if="busy" class="m-0">{{ formatMessage(messages.loading) }}</p>
            <p v-if="error" class="m-0 whitespace-pre-wrap" role="alert">{{ error }}</p>
            <p v-if="resultText" class="m-0" role="status">{{ resultText }}</p>
            <p v-if="!busy && !error && !files.length" class="m-0">
                {{ formatMessage(messages.empty) }}
            </p>
            <template v-if="plan">
                <p class="m-0">
                    {{
                        formatMessage(messages.warning, {
                            count: plan.files.filter((file) => file.rollbackAvailable).length,
                        })
                    }}
                </p>
                <p v-for="file in plan.files" :key="file.id" class="m-0 break-all">
                    {{ file.relativePath }}
                </p>
            </template>
            <template v-else>
                <div
                    v-for="file in files"
                    :key="file.id"
                    class="flex items-start gap-3 rounded-lg border border-surface-4 bg-surface-2 p-3"
                >
                    <Checkbox
                        :model-value="selected.includes(file.id)"
                        :disabled="busy || !file.recognized || !file.exists"
                        :description="file.relativePath"
                        @update:model-value="select(file.id, $event)"
                    />
                    <div class="min-w-0 flex-1">
                        <p class="m-0 break-all font-semibold">{{ file.relativePath }}</p>
                        <p class="m-0 text-sm">
                            {{
                                formatMessage(
                                    !file.exists
                                        ? messages.missing
                                        : !file.recognized
                                          ? messages.unknown
                                          : file.rollbackAvailable
                                            ? messages.valid
                                            : file.historyId
                                              ? messages.invalid
                                              : messages.orphan,
                                )
                            }}
                        </p>
                        <p v-if="file.exists" class="m-0 text-sm tabular-nums">
                            {{
                                formatMessage(messages.fileSize, { size: file.integrity.size ?? 0 })
                            }}
                        </p>
                    </div>
                </div>
                <p v-if="selected.length" class="m-0">
                    {{ formatMessage(messages.summary, { count: selected.length, size }) }}
                </p>
            </template>
        </div>
        <template #actions>
            <Button :disabled="cleaning" @click="modal?.hide()">{{
                formatMessage(messages.close)
            }}</Button>
            <Button v-if="plan" :disabled="busy" @click="plan = null">{{
                formatMessage(messages.back)
            }}</Button>
            <Button v-else :disabled="busy" @click="scan()">{{
                formatMessage(messages.scan)
            }}</Button>
            <Button v-if="plan" color="red" :disabled="busy" @click="cleanup()">{{
                formatMessage(messages.confirm)
            }}</Button>
            <Button v-else :disabled="busy || !selected.length" @click="review()">{{
                formatMessage(messages.review)
            }}</Button>
        </template>
    </NewModal>
</template>
