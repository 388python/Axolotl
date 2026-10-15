<script setup lang="ts">
import { RefreshCwIcon } from '@modrinth/assets'
import { Button, useFormatBytes, useVIntl } from '@modrinth/ui'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { openStoragePaths, scanStorage } from '@/helpers/storage'
import { previewCacheUsage } from '@/helpers/storage/preview-cache-usage'

import SettingsSection from './SettingsSection.vue'
import MetaStorageCleanup from './storage/MetaStorageCleanup.vue'
import type { StorageNode, StorageSize, StorageTree } from './storage/storageData'
import { storageLabels, storageMessages } from './storage/storageMessages'
import StorageTreeNode from './storage/StorageTreeNode.vue'

const { formatMessage } = useVIntl()
const formatBytes = useFormatBytes()
const router = useRouter()

const tree = ref<StorageTree | null>(null)
const loading = ref(true)
const lastUpdated = ref<Date | null>(null)
const scanError = ref(false)
const previewBytes = ref<number | null>(null)
let requestId = 0

const categories = computed(() => tree.value?.categories ?? [])

const mainCategories = computed<StorageNode[]>(() => {
    const list = categories.value.filter(
        (category) => category.type !== 'other' && category.size.actual + category.size.symlink > 0,
    )
    const rootOther = tree.value?.rootOther
    if (rootOther && rootOther.size.actual + rootOther.size.symlink > 0) {
        list.push(rootOther)
    }
    return list
})

const categoryLabels = storageLabels
const palette = [
    '--color-green',
    '--color-orange',
    '--color-purple',
    '--color-blue',
    '--color-red',
    '--color-text-tertiary',
]
function categoryColor(index: number) {
    return `var(${palette[index % palette.length]})`
}

const hoveredId = ref<string | null>(null)

function sizeTotal(size: StorageSize) {
    return size.actual + size.symlink
}

function formatSize(size: StorageSize) {
    const actual = formatBytes(size.actual)
    if (size.symlink <= 0) return actual
    return `${actual} + ${formatBytes(size.symlink)}`
}

async function loadStorage(force: boolean) {
    const current = ++requestId
    loading.value = true
    scanError.value = false
    try {
        const [snapshot, previews] = await Promise.all([
            scanStorage(force),
            previewCacheUsage().catch(() => null),
        ])
        if (current !== requestId) return false
        previewBytes.value = previews
        tree.value = snapshot
        lastUpdated.value = snapshot.scannedAt ? new Date(snapshot.scannedAt) : new Date()
        return true
    } catch (error) {
        if (current !== requestId) return false
        scanError.value = true
        console.warn('[storage] Scan failed', error)
        return false
    } finally {
        if (current === requestId) loading.value = false
    }
}

async function updateStorage() {
    return loadStorage(true)
}

onMounted(() => void loadStorage(false))
onUnmounted(() => {
    requestId += 1
})

function launcherRouteFor(node: StorageNode, instanceId: string): string | null {
    const encodedId = encodeURIComponent(instanceId)
    switch (node.type) {
        case 'instance':
        case 'mods':
            return `/instance/${encodedId}`
        case 'saves':
        case 'world':
            return `/instance/${encodedId}/worlds`
        case 'screenshots':
            return `/instance/${encodedId}/screenshots`
        default:
            return null
    }
}

async function tryNavigate(path: string): Promise<boolean> {
    try {
        await router.push(path)
        return true
    } catch (error) {
        console.warn('[storage] Launcher navigation failed, falling back to filesystem', error)
        return false
    }
}

async function openNodePaths(node: StorageNode) {
    if (node.paths.length === 0) return
    try {
        const result = await openStoragePaths(node.paths)
        for (const failure of result.failed) {
            console.warn(`[storage] Failed to open path: ${failure.path}`, failure.reason)
        }
    } catch (error) {
        console.warn('[storage] Failed to open storage paths', error)
    }
}

async function handleAction(node: StorageNode) {
    const instanceId = node.instance_id
    const route = instanceId ? launcherRouteFor(node, instanceId) : null

    if (route && (await tryNavigate(route))) return

    await openNodePaths(node)
}

interface ChartItem {
    id: string
    category: StorageNode
    label: string
    sizeBytes: number
    formattedSize: string
    color: string
    isSymlink: boolean
    startAngle: number
    endAngle: number
    pathData: string
    percentText: string
}

// 极其精准的 SVG Path 圆弧/环形生成算法（彻底解决 SVG circle dashoffset 错位乱套问题）
function getRingPath(
    cx: number,
    cy: number,
    rInner: number,
    rOuter: number,
    startAngle: number,
    endAngle: number,
) {
    // 防止 100% 比例下起点终点重合导致无法绘制
    const angleDiff = endAngle - startAngle
    const safeEndAngle = angleDiff >= 360 ? startAngle + 359.999 : endAngle

    const rad = (deg: number) => (deg - 90) * (Math.PI / 180)

    const x1 = cx + rOuter * Math.cos(rad(startAngle))
    const y1 = cy + rOuter * Math.sin(rad(startAngle))
    const x2 = cx + rOuter * Math.cos(rad(safeEndAngle))
    const y2 = cy + rOuter * Math.sin(rad(safeEndAngle))

    const x3 = cx + rInner * Math.cos(rad(safeEndAngle))
    const y3 = cy + rInner * Math.sin(rad(safeEndAngle))
    const x4 = cx + rInner * Math.cos(rad(startAngle))
    const y4 = cy + rInner * Math.sin(rad(startAngle))

    const largeArc = angleDiff > 180 ? 1 : 0

    return [
        `M ${x1} ${y1}`,
        `A ${rOuter} ${rOuter} 0 ${largeArc} 1 ${x2} ${y2}`,
        `L ${x3} ${y3}`,
        `A ${rInner} ${rInner} 0 ${largeArc} 0 ${x4} ${y4}`,
        'Z',
    ].join(' ')
}

const chartSlices = computed(() => {
    const items: {
        id: string
        category: StorageNode
        label: string
        sizeBytes: number
        formattedSize: string
        color: string
        isSymlink: boolean
    }[] = []

    // 拆分实体与软链接
    for (const [index, cat] of mainCategories.value.entries()) {
        const baseLabel = formatMessage(categoryLabels[cat.type])

        if (cat.size.actual > 0) {
            items.push({
                id: `${cat.id}-actual`,
                category: cat,
                label: baseLabel,
                sizeBytes: cat.size.actual,
                formattedSize: formatBytes(cat.size.actual),
                color: categoryColor(index),
                isSymlink: false,
            })
        }

        if (cat.size.symlink > 0) {
            items.push({
                id: `${cat.id}-symlink`,
                category: cat,
                label: `${baseLabel} (${formatMessage(storageMessages.symlinkLabel)})`,
                sizeBytes: cat.size.symlink,
                formattedSize: formatBytes(cat.size.symlink),
                color: categoryColor(index),
                isSymlink: true,
            })
        }
    }

    const total = items.reduce((acc, item) => acc + item.sizeBytes, 0) || 1
    let currentAngle = 0

    return items.map((item) => {
        const ratio = item.sizeBytes / total
        const angle = ratio * 360
        const startAngle = currentAngle
        const endAngle = currentAngle + angle
        currentAngle = endAngle

        return {
            ...item,
            startAngle,
            endAngle,
            percentText: `${(ratio * 100).toFixed(1)}%`,
            pathData: getRingPath(50, 50, 26, 48, startAngle, endAngle), // 外径48，内径26，加粗环形
        } as ChartItem
    })
})

function formatDateTime(date: Date) {
    return new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
    }).format(date)
}
</script>

<template>
    <div
        id="settings-target-storage-overview"
        tabindex="-1"
        class="flex w-full flex-col gap-3 text-[var(--color-text-primary)]"
    >
        <div v-if="loading && !tree" class="flex flex-col gap-3" aria-busy="true">
            <span>{{ formatMessage(storageMessages.scanning) }}</span>
            <div v-for="i in 4" :key="i" class="h-8 animate-pulse rounded-lg bg-surface-3" />
        </div>
        <div v-if="scanError" class="flex items-center justify-between gap-3" role="alert">
            <span>{{ formatMessage(storageMessages.scanFailed) }}</span>
            <Button :loading="loading" @click="updateStorage">{{
                formatMessage(storageMessages.retry)
            }}</Button>
        </div>

        <template v-if="tree">
            <!-- 顶部面板：总览 + 更新按钮 + 饼图 -->
            <section
                class="flex flex-wrap items-center justify-between gap-6 overflow-hidden rounded-[var(--radius-md)] border border-solid border-surface-4 bg-surface-3 p-4"
            >
                <div class="flex shrink-0 flex-col gap-1">
                    <h2 class="m-0 text-sm font-medium text-[var(--color-text-tertiary)]">
                        {{ formatMessage(storageMessages.total) }}
                    </h2>

                    <div class="flex flex-col gap-1 tabular-nums">
                        <span
                            class="text-3xl font-bold leading-[1.1] text-[var(--color-text-primary)]"
                            >{{ formatBytes(tree.total.actual) }}</span
                        >
                        <span
                            v-if="tree.total.symlink > 0"
                            class="text-sm font-medium text-[var(--color-text-tertiary)]"
                        >
                            + {{ formatBytes(tree.total.symlink) }} ({{
                                formatMessage(storageMessages.symlinkLabel)
                            }})
                        </span>
                    </div>

                    <div class="mt-2 flex flex-col items-start gap-1">
                        <Button :loading="loading" @click="updateStorage">
                            <RefreshCwIcon />
                            {{
                                formatMessage(
                                    loading ? storageMessages.updating : storageMessages.update,
                                )
                            }}
                        </Button>
                        <span
                            v-if="lastUpdated"
                            class="flex flex-col text-xs text-[var(--color-text-tertiary)]"
                        >
                            <span>{{ formatMessage(storageMessages.lastUpdatedLabel) }}</span>
                            <span class="tabular-nums">{{ formatDateTime(lastUpdated) }}</span>
                        </span>
                    </div>
                </div>

                <div
                    v-if="mainCategories.length > 0"
                    class="flex min-w-0 flex-wrap items-center gap-6"
                >
                    <div class="relative size-32 shrink-0">
                        <svg class="h-full w-full overflow-visible" viewBox="0 0 100 100">
                            <path
                                v-for="slice in chartSlices"
                                :key="slice.id"
                                :d="slice.pathData"
                                :fill="slice.color"
                                class="pie-path"
                                :class="{
                                    'is-hovered': hoveredId === slice.id,
                                    'is-dimmed': hoveredId !== null && hoveredId !== slice.id,
                                }"
                                @mouseenter="hoveredId = slice.id"
                                @mouseleave="hoveredId = null"
                                @click="handleAction(slice.category)"
                            />
                        </svg>
                    </div>

                    <div class="grid min-w-0 grid-cols-1 gap-2 sm:grid-cols-2">
                        <button
                            v-for="slice in chartSlices"
                            :key="slice.id"
                            type="button"
                            class="legend-item flex cursor-pointer items-start gap-2 rounded-md border-0 bg-transparent px-2 py-1 text-left text-inherit focus-visible:outline-brand"
                            :class="{
                                'is-hovered': hoveredId === slice.id,
                                'is-dimmed': hoveredId !== null && hoveredId !== slice.id,
                            }"
                            @mouseenter="hoveredId = slice.id"
                            @mouseleave="hoveredId = null"
                            @click="handleAction(slice.category)"
                        >
                            <span
                                class="legend-dot mt-1 h-2.5 w-2.5 shrink-0 rounded-full"
                                :class="{ 'is-symlink-dot': slice.isSymlink }"
                                :style="{ backgroundColor: slice.color }"
                            />

                            <div class="flex min-w-0 flex-col leading-tight">
                                <span
                                    class="break-words text-sm font-semibold text-[var(--color-text-primary)]"
                                >
                                    {{ slice.label }}
                                </span>
                                <span
                                    class="whitespace-nowrap text-xs tabular-nums text-[var(--color-text-tertiary)]"
                                    >{{ slice.formattedSize }}</span
                                >
                                <span
                                    class="text-xs tabular-nums text-[var(--color-text-tertiary)]"
                                    >{{ slice.percentText }}</span
                                >
                            </div>
                        </button>
                    </div>
                </div>
            </section>

            <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(storageMessages.measurementHelp) }}
            </p>
            <details v-if="tree.warnings.length" class="mt-3 text-sm">
                <summary>{{ formatMessage(storageMessages.incompleteScan) }}</summary>
                <ul>
                    <li v-for="warning in tree.warnings" :key="warning" class="break-all">
                        {{ warning }}
                    </li>
                </ul>
            </details>
            <SettingsSection v-for="category in mainCategories" :key="category.id" class="mt-6">
                <template #header>
                    <div class="flex w-full items-center justify-between gap-3">
                        <span>{{ formatMessage(categoryLabels[category.type]) }}</span>
                        <span class="text-sm tabular-nums">{{ formatSize(category.size) }}</span>
                    </div>
                </template>
                <div class="p-4">
                    <StorageTreeNode
                        :node="category"
                        :depth="0"
                        :parent-total="sizeTotal(tree.total)"
                        @action="handleAction"
                    />
                </div>
            </SettingsSection>
            <SettingsSection v-if="previewBytes !== null">
                <template #header>{{ formatMessage(storageMessages.previewCache) }}</template>
                <div class="flex flex-col gap-2 p-4">
                    <span class="font-semibold tabular-nums">{{ formatBytes(previewBytes) }}</span>
                    <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(storageMessages.previewCacheHelp) }}
                    </p>
                </div>
            </SettingsSection>
            <SettingsSection>
                <template #header>{{ formatMessage(storageMessages.metaData) }}</template>
                <div class="p-4"><MetaStorageCleanup :refresh-storage="updateStorage" /></div>
            </SettingsSection>
            <SettingsSection v-if="tree.external.length" class="mt-6">
                <template #header>{{ formatMessage(storageMessages.externalData) }}</template>
                <div class="flex flex-col gap-3 p-4">
                    <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(storageMessages.externalHelp) }}
                    </p>
                    <StorageTreeNode
                        v-for="node in tree.external"
                        :key="node.id"
                        :node="node"
                        :depth="0"
                        :parent-total="sizeTotal(node.size)"
                        @action="handleAction"
                    />
                </div>
            </SettingsSection>
        </template>
    </div>
</template>

<style scoped>
.pie-path {
    cursor: pointer;
    transition: opacity 150ms ease;
}
.pie-path.is-dimmed {
    opacity: 0.3;
}
.legend-item {
    transition:
        background-color 120ms ease,
        opacity 150ms ease;
}
.legend-item.is-hovered {
    background: var(--surface-4);
}
.legend-item.is-dimmed {
    opacity: 0.3;
}
.legend-dot.is-symlink-dot {
    border-radius: 2px;
}
@media (prefers-reduced-motion: reduce) {
    .pie-path,
    .legend-item {
        transition: none;
    }
}
</style>
