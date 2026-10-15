<script setup lang="ts">
import { Button, Checkbox, NewModal, useFormatBytes, useVIntl } from '@modrinth/ui'
import { computed, nextTick, ref } from 'vue'

import {
    analyzeMeta,
    cleanMeta,
    type MetaAnalysis,
    type MetaCleanupResult,
} from '@/helpers/storage'

import { storageLabels, storageMessages } from './storageMessages'

const props = defineProps<{ refreshStorage: () => Promise<boolean> }>()
const { formatMessage } = useVIntl()
const formatBytes = useFormatBytes()
const analysis = ref<MetaAnalysis | null>(null)
const selected = ref<string[]>([])
const analyzing = ref(false)
const cleaning = ref(false)
const error = ref<'analyzeFailed' | 'cleanFailed' | 'refreshFailed' | null>(null)
const result = ref<MetaCleanupResult | null>(null)
const errorDetail = ref('')
const cleanupCompleted = ref(false)
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const selectedSize = computed(
    () =>
        analysis.value?.groups.reduce(
            (sum, group) => sum + (selected.value.includes(group.id) ? group.unused : 0),
            0,
        ) ?? 0,
)

function groupLabel(category: string, name: string) {
    const label =
        category in storageLabels
            ? formatMessage(storageLabels[category as keyof typeof storageLabels])
            : category
    return name ? `${label} / ${name}` : label
}

function select(id: string, checked: boolean) {
    selected.value = checked
        ? [...selected.value, id]
        : selected.value.filter((value) => value !== id)
}

async function analyze() {
    if (analyzing.value || cleaning.value) return
    analyzing.value = true
    error.value = null
    errorDetail.value = ''
    try {
        const next = await analyzeMeta()
        analysis.value = next
        selected.value = []
    } catch (cause) {
        error.value = 'analyzeFailed'
        console.warn('[storage] Metadata analysis failed', cause)
    } finally {
        analyzing.value = false
    }
}

async function clean() {
    if (!analysis.value || cleaning.value || !selectedSize.value) return
    cleaning.value = true
    error.value = null
    errorDetail.value = ''
    try {
        result.value = await cleanMeta(analysis.value.id, selected.value)
        analysis.value = null
        selected.value = []
        cleanupCompleted.value = true
        await refreshAfterCleanup()
    } catch (cause) {
        error.value = 'cleanFailed'
        errorDetail.value = cause instanceof Error ? cause.message : String(cause)
        console.warn('[storage] Metadata cleanup failed', cause)
    } finally {
        cleaning.value = false
    }
}
async function refreshAfterCleanup() {
    cleaning.value = true
    error.value = null
    try {
        const refreshed = await props.refreshStorage()
        if (!refreshed) {
            error.value = 'refreshFailed'
            return
        }
        cleaning.value = false
        await nextTick()
        modal.value?.hide()
    } catch {
        error.value = 'refreshFailed'
    } finally {
        cleaning.value = false
    }
}

function review(event: MouseEvent) {
    cleanupCompleted.value = false
    modal.value?.show(event)
}
</script>

<template>
    <div class="flex flex-col gap-3">
        <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
            {{ formatMessage(storageMessages.metaHelp) }}
        </p>
        <div class="flex flex-wrap items-center gap-3">
            <Button :loading="analyzing" :disabled="cleaning" @click="analyze">
                {{
                    formatMessage(
                        analyzing ? storageMessages.analyzingMeta : storageMessages.analyzeMeta,
                    )
                }}
            </Button>
            <Button
                v-if="selectedSize > 0"
                type="colored-text"
                color="red"
                :disabled="analyzing || cleaning"
                @click="review"
            >
                {{ formatMessage(storageMessages.cleanSelected) }}
            </Button>
        </div>
        <p v-if="error && !cleaning" class="m-0 text-sm text-red" role="alert">
            {{ formatMessage(storageMessages[error]) }}
        </p>
        <div v-if="result" role="status" class="flex flex-col gap-2 text-sm">
            <p class="m-0">
                {{
                    formatMessage(storageMessages.cleanResult, {
                        count: result.removed_files,
                        size: formatBytes(result.removed_bytes),
                        skipped: result.skipped_files,
                    })
                }}
            </p>
            <details v-if="result.failures.length">
                <summary>{{ formatMessage(storageMessages.cleanFailures) }}</summary>
                <ul>
                    <li v-for="failure in result.failures" :key="failure" class="break-all">
                        {{ failure }}
                    </li>
                </ul>
            </details>
        </div>
        <template v-if="analysis">
            <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(storageMessages.refreshRequired) }}
            </p>
            <details v-if="analysis.warnings.length" class="text-sm">
                <summary>{{ formatMessage(storageMessages.metaWarnings) }}</summary>
                <ul>
                    <li v-for="warning in analysis.warnings" :key="warning" class="break-all">
                        {{ warning }}
                    </li>
                </ul>
            </details>
            <div class="overflow-x-auto" :aria-busy="analyzing || undefined">
                <table class="w-full border-collapse text-left text-sm">
                    <thead>
                        <tr class="border-0 border-b border-solid border-surface-4">
                            <th class="p-2">{{ formatMessage(storageMessages.metaGroup) }}</th>
                            <th class="p-2 text-right">
                                {{ formatMessage(storageMessages.used) }}
                            </th>
                            <th class="p-2 text-right">
                                {{ formatMessage(storageMessages.unused) }}
                            </th>
                            <th class="p-2 text-right">
                                {{ formatMessage(storageMessages.unknown) }}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr
                            v-for="group in analysis.groups"
                            :key="group.id"
                            class="border-0 border-b border-solid border-surface-4"
                        >
                            <td class="p-2">
                                <Checkbox
                                    v-if="group.unused > 0"
                                    :model-value="selected.includes(group.id)"
                                    :disabled="analyzing || cleaning"
                                    :description="
                                        formatMessage(storageMessages.selectGroup, {
                                            name: groupLabel(group.category, group.name),
                                        })
                                    "
                                    @update:model-value="select(group.id, $event)"
                                >
                                    <span class="break-all">{{
                                        groupLabel(group.category, group.name)
                                    }}</span>
                                </Checkbox>
                                <span v-else class="break-all">{{
                                    groupLabel(group.category, group.name)
                                }}</span>
                            </td>
                            <td class="whitespace-nowrap p-2 text-right tabular-nums">
                                {{ formatBytes(group.used) }}
                            </td>
                            <td class="whitespace-nowrap p-2 text-right tabular-nums">
                                {{ formatBytes(group.unused) }}
                            </td>
                            <td class="whitespace-nowrap p-2 text-right tabular-nums">
                                {{ formatBytes(group.unknown) }}
                            </td>
                        </tr>
                    </tbody>
                </table>
            </div>
            <p
                v-if="selectedSize > 0 || !analysis.groups.some((group) => group.unused > 0)"
                class="m-0 text-sm"
            >
                {{
                    selectedSize > 0
                        ? formatMessage(storageMessages.selectedSize, {
                              count: selected.length,
                              size: formatBytes(selectedSize),
                          })
                        : formatMessage(storageMessages.noUnused)
                }}
            </p>
        </template>
    </div>
    <NewModal
        ref="modal"
        :header="formatMessage(storageMessages.cleanTitle)"
        :disable-close="cleaning"
        width="min(36rem, 90vw)"
    >
        <p v-if="!cleanupCompleted">
            {{ formatMessage(storageMessages.cleanConfirm, { size: formatBytes(selectedSize) }) }}
        </p>
        <p v-else-if="result">
            {{
                formatMessage(storageMessages.cleanResult, {
                    count: result.removed_files,
                    size: formatBytes(result.removed_bytes),
                    skipped: result.skipped_files,
                })
            }}
        </p>
        <p v-if="error === 'refreshFailed'" role="alert">
            {{ formatMessage(storageMessages.refreshFailed) }}
        </p>
        <div v-if="error === 'cleanFailed'" role="alert" class="flex flex-col gap-2 text-red">
            <p class="m-0">{{ formatMessage(storageMessages.cleanFailed) }}</p>
            <p v-if="errorDetail" class="m-0 break-words text-sm">{{ errorDetail }}</p>
        </div>
        <template #actions>
            <Button :disabled="cleaning" @click="modal?.hide()">{{
                formatMessage(storageMessages.cancel)
            }}</Button>
            <Button v-if="cleanupCompleted" :loading="cleaning" @click="refreshAfterCleanup">{{
                formatMessage(storageMessages.retry)
            }}</Button>
            <Button v-else type="colored" color="red" :loading="cleaning" @click="clean">{{
                formatMessage(cleaning ? storageMessages.cleaning : storageMessages.cleanSelected)
            }}</Button>
        </template>
    </NewModal>
</template>
