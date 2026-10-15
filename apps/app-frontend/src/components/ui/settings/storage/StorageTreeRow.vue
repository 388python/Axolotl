<script setup lang="ts">
import { ChevronRightIcon, FileIcon, FolderIcon, FolderOpenIcon } from '@modrinth/assets'
import { useFormatBytes, useVIntl } from '@modrinth/ui'
import { computed } from 'vue'

import type { StorageNode } from './storageData'
import { storageLabels, storageMessages } from './storageMessages'

defineOptions({ name: 'StorageTreeRow' })

const props = defineProps<{
    node: StorageNode
    depth: number
    parentTotal: number
    expanded: boolean
}>()

const emit = defineEmits<{
    action: [node: StorageNode]
}>()

const { formatMessage } = useVIntl()
const formatBytes = useFormatBytes()

const typeLabels = storageLabels

const hasChildren = computed(() => (props.node.children?.length ?? 0) > 0)
const isDirectory = computed(() => (props.node.paths[0]?.kind ?? 'directory') !== 'file')

const actualShare = computed(() =>
    props.parentTotal > 0 ? props.node.size.actual / props.parentTotal : 0,
)

const symShare = computed(() =>
    props.parentTotal > 0 ? props.node.size.symlink / props.parentTotal : 0,
)

const totalShare = computed(() => Math.min(1, actualShare.value + symShare.value))

const percent = computed(() => Math.round(totalShare.value * 100))

const displayLabel = computed(() => props.node.name ?? formatMessage(typeLabels[props.node.type]))

const tooltipText = computed(() => props.node.paths.map((path) => path.path).join('\n'))

const nameTooltip = computed(() => ({
    content: tooltipText.value,
    popperClass: 'storage-tooltip',
}))

const actualSizeTooltip = computed(() =>
    formatMessage(storageMessages.actualSizeTooltip, {
        size: formatBytes(props.node.size.actual),
    }),
)

const symlinkSizeTooltip = computed(() =>
    formatMessage(storageMessages.symlinkSizeTooltip, {
        size: formatBytes(props.node.size.symlink),
    }),
)

const progressTitle = computed(() =>
    props.node.size.symlink > 0
        ? `${actualSizeTooltip.value}\n${symlinkSizeTooltip.value}`
        : actualSizeTooltip.value,
)

const progressTooltip = computed(() => ({
    content: progressTitle.value,
    popperClass: 'storage-tooltip',
}))

const progressLabel = computed(() =>
    props.node.size.symlink > 0
        ? `${actualSizeTooltip.value} ${symlinkSizeTooltip.value}`
        : actualSizeTooltip.value,
)

const actualSizeText = computed(() => formatBytes(props.node.size.actual))
const symlinkSizeText = computed(() => formatBytes(props.node.size.symlink))

const openActionLabel = computed(() => formatMessage(storageMessages.openAction))
</script>

<template>
    <div
        class="tree-row flex min-h-8 select-none items-center justify-between border-0 bg-transparent px-2 py-1 hover:bg-surface-3"
        :class="{ clickable: hasChildren, 'is-expanded': expanded }"
    >
        <!-- 左侧树层级与名称部分 -->
        <div class="flex h-full min-w-0 flex-1 items-center">
            <!-- 根据 depth 生成层级缩进和导轨线 -->
            <div class="indent-guides flex h-full shrink-0" aria-hidden="true">
                <span v-for="i in depth" :key="i" class="guide-line relative block h-full w-5" />
            </div>

            <!-- 展开/折叠 箭头（装饰性，由原生的 <summary> 负责展开） -->
            <span
                v-if="hasChildren"
                class="chevron-icon inline-flex h-5 w-5 shrink-0 items-center justify-center p-0 text-[var(--color-text-tertiary)]"
                aria-hidden="true"
            >
                <ChevronRightIcon class="size-3.5" />
            </span>
            <span v-else class="w-5 shrink-0" aria-hidden="true" />

            <!-- 文件/文件夹按钮：打开位置（不展开树） -->
            <button
                v-tooltip="openActionLabel"
                type="button"
                class="node-type-btn mr-1.5 inline-flex cursor-pointer items-center justify-center rounded border-0 bg-transparent p-0 text-[var(--color-text-tertiary)] hover:bg-surface-3 hover:text-[var(--color-text-primary)]"
                :aria-label="`${displayLabel}: ${openActionLabel}`"
                @click.stop="emit('action', node)"
            >
                <FolderOpenIcon v-if="hasChildren && expanded" class="size-4 shrink-0 text-brand" />
                <FolderIcon v-else-if="isDirectory" class="size-4 shrink-0" />
                <FileIcon v-else class="size-4 shrink-0" />
            </button>

            <!-- 节点名称 -->
            <span
                v-tooltip="nameTooltip"
                class="min-w-0 truncate text-sm font-normal text-[var(--color-text-primary)]"
            >
                {{ displayLabel }}
            </span>

            <span
                v-if="node.count !== undefined"
                class="ml-1.5 h-4 shrink-0 rounded bg-surface-3 px-1 text-xs leading-4 tabular-nums text-[var(--color-text-tertiary)]"
            >
                {{ node.count }}
            </span>
        </div>

        <!-- 右侧数据与进度列 -->
        <div class="ml-4 flex shrink-0 items-center gap-4">
            <div
                class="min-w-20 whitespace-nowrap text-right text-sm tabular-nums text-[var(--color-text-default)]"
            >
                {{ actualSizeText }}
                <span v-if="node.size.symlink > 0" class="text-[var(--color-text-tertiary)]">
                    + {{ symlinkSizeText }}
                </span>
            </div>

            <div
                class="w-9 max-sm:hidden text-right text-xs tabular-nums text-[var(--color-text-tertiary)]"
            >
                {{ percent }}%
            </div>

            <progress
                v-tooltip="progressTooltip"
                class="storage-progress h-1 w-24 shrink-0 appearance-none overflow-hidden rounded-full border-0 bg-surface-3"
                :value="totalShare"
                :max="1"
                :aria-label="progressLabel"
            />
        </div>
    </div>
</template>

<style scoped>
.tree-row {
    transition: background-color 0.1s ease;
}
.tree-row.clickable {
    cursor: pointer;
}
.guide-line::before {
    content: '';
    position: absolute;
    left: 0.5rem;
    top: 0;
    bottom: 0;
    width: 1px;
    background-color: var(--surface-5);
}
.tree-row.is-expanded .guide-line:last-child::before {
    background-color: var(--color-brand);
}
.chevron-icon {
    transition: transform 0.15s ease;
}
.tree-row.is-expanded .chevron-icon {
    transform: rotate(90deg);
}
.node-type-btn:focus-visible {
    outline: 2px solid var(--color-brand);
    outline-offset: -1px;
}
.storage-progress::-webkit-progress-bar {
    background: var(--surface-3);
    border-radius: 9999px;
}
.storage-progress::-webkit-progress-value {
    background: var(--color-brand);
    border-radius: 9999px;
}
.storage-progress::-moz-progress-bar {
    background: var(--color-brand);
    border-radius: 9999px;
}
@media (max-width: 900px) {
    .storage-progress {
        display: none;
    }
}
@media (prefers-reduced-motion: reduce) {
    .tree-row,
    .chevron-icon {
        transition: none;
    }
}
:global(.tooltip-popper.storage-tooltip) {
    white-space: pre-line;
    max-width: 22rem;
}
</style>
