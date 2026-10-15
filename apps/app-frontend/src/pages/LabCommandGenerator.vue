<script setup lang="ts">
import { BlocksIcon, BookmarkIcon, CheckCircleIcon, SparklesIcon, XIcon } from '@modrinth/assets'
import { Button, Combobox, type ComboboxOption, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import CommandGeneratorArgument from '@/components/lab/command-generator/CommandGeneratorArgument.vue'
import CommandGeneratorExecuteChain, {
    type ExecuteChainState,
} from '@/components/lab/command-generator/CommandGeneratorExecuteChain.vue'
import { findGuide } from '@/lab/command-generator/commands/chinese-guide.ts'
import {
    CATEGORY_LABELS,
    getAllCommands,
    findCommand,
    getCurrentVersion,
    initializeRegistry,
    switchVersion,
} from '@/lab/command-generator/commands/registry.ts'
import type { CommandCategory } from '@/lab/command-generator/commands/types.ts'
import {
    buildCommand,
    getDefaultValues,
    validateCommand,
} from '@/lab/command-generator/generator.ts'
import { loadManifest } from '@/lab/command-generator/loader.ts'
import {
    applyPresetToValues,
    deleteUserPreset,
    getPresetsForCommand,
    getUserPresets,
    saveUserPreset,
    type UserPreset,
} from '@/lab/command-generator/presets.ts'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: { id: 'app.lab.command-generator.title', defaultMessage: 'Command generator' },
    subtitle: {
        id: 'app.lab.command-generator.subtitle',
        defaultMessage: 'Build Minecraft Java Edition commands visually.',
    },
    selectVersion: {
        id: 'app.lab.command-generator.select-version',
        defaultMessage: 'Game version',
    },
    selectCommand: {
        id: 'app.lab.command-generator.select-command',
        defaultMessage: 'Choose a command',
    },
    selectPlaceholder: {
        id: 'app.lab.command-generator.select-placeholder',
        defaultMessage: 'Search a command…',
    },
    guideTitle: { id: 'app.lab.command-generator.guide-title', defaultMessage: 'How to use' },
    guideExamples: {
        id: 'app.lab.command-generator.guide-examples',
        defaultMessage: 'Examples',
    },
    guideTips: { id: 'app.lab.command-generator.guide-tips', defaultMessage: 'Tips' },
    parametersTitle: {
        id: 'app.lab.command-generator.parameters-title',
        defaultMessage: 'Parameters',
    },
    parametersEmpty: {
        id: 'app.lab.command-generator.parameters-empty',
        defaultMessage: 'This command takes no arguments.',
    },
    generate: { id: 'app.lab.command-generator.generate', defaultMessage: 'Generate' },
    copy: { id: 'app.lab.command-generator.copy', defaultMessage: 'Copy' },
    copied: { id: 'app.lab.command-generator.copied', defaultMessage: 'Copied' },
    result: {
        id: 'app.lab.command-generator.result',
        defaultMessage: 'Generated command',
    },
    savePreset: {
        id: 'app.lab.command-generator.save-preset',
        defaultMessage: 'Save as preset',
    },
    presetName: {
        id: 'app.lab.command-generator.preset-name',
        defaultMessage: 'Preset name',
    },
    myPresets: {
        id: 'app.lab.command-generator.my-presets',
        defaultMessage: 'My presets',
    },
    filterCategory: {
        id: 'app.lab.command-generator.filter-category',
        defaultMessage: 'Category',
    },
    filterPermission: {
        id: 'app.lab.command-generator.filter-permission',
        defaultMessage: 'Permission level',
    },
    filterAll: {
        id: 'app.lab.command-generator.filter-all',
        defaultMessage: 'All',
    },
    globalPresets: {
        id: 'app.lab.command-generator.global-presets',
        defaultMessage: 'All presets',
    },
    globalPresetsEmpty: {
        id: 'app.lab.command-generator.global-presets-empty',
        defaultMessage: 'No saved presets yet.',
    },
    globalPresetsHint: {
        id: 'app.lab.command-generator.global-presets-hint',
        defaultMessage: 'Click a preset to jump to its version and command.',
    },
})

const PERMISSION_NAMES: Record<number, string> = {
    0: '所有人',
    1: '管理员',
    2: 'OP',
    3: '服务器控制台',
    4: '仅单人',
}

// ============= 状态 =============
const versionOptions = ref<ComboboxOption[]>([])
const selectedVersion = ref('')
const selectedCommandId = ref('')
const searchQuery = ref('')
const filterCategory = ref<string>('')
const filterPermission = ref<string>('')
const values = ref<Record<string, string | number | boolean>>({})
const executeChainOutput = ref('')
const executeChainRef = ref<InstanceType<typeof CommandGeneratorExecuteChain> | null>(null)
const result = ref('')
const errors = ref<string[]>([])
const copied = ref(false)
const presetNameInput = ref('')
const showPresetInput = ref(false)
const toast = ref('')
const showGlobalPresets = ref(false)

const userPresets = ref<UserPreset[]>([])

// ============= 筛选器选项 =============
const categoryOptions = computed<ComboboxOption[]>(() => [
    { value: '', label: formatMessage(messages.filterAll) },
    ...(Object.keys(CATEGORY_LABELS) as CommandCategory[]).map((key) => ({
        value: key,
        label: CATEGORY_LABELS[key],
    })),
])

const permissionOptions = computed<ComboboxOption[]>(() => [
    { value: '', label: formatMessage(messages.filterAll) },
    { value: '0', label: `0 · ${PERMISSION_NAMES[0]}` },
    { value: '1', label: `1 · ${PERMISSION_NAMES[1]}` },
    { value: '2', label: `2 · ${PERMISSION_NAMES[2]}` },
    { value: '3', label: `3 · ${PERMISSION_NAMES[3]}` },
    { value: '4', label: `4 · ${PERMISSION_NAMES[4]}` },
])

const versionLabelMap = computed<Record<string, string>>(() => {
    const map: Record<string, string> = {}
    for (const opt of versionOptions.value) {
        map[String(opt.value)] = String(opt.label)
    }
    return map
})

// ============= 命令列表（带筛选） =============
const filteredCommands = computed(() => {
    let list = getAllCommands()
    if (filterCategory.value) {
        list = list.filter((c) => c.category === filterCategory.value)
    }
    if (filterPermission.value !== '') {
        list = list.filter((c) => String(c.permissionLevel) === filterPermission.value)
    }
    return list
})

const commandOptions = computed<ComboboxOption[]>(() =>
    filteredCommands.value.map((cmd) => ({
        value: cmd.id,
        label: cmd.name,
        description: cmd.description,
        category: CATEGORY_LABELS[cmd.category],
    })),
)

// ============= 当前选中 =============
const currentCommand = computed(() =>
    selectedCommandId.value ? (findCommand(selectedCommandId.value) ?? null) : null,
)

const currentGuide = computed(() =>
    selectedCommandId.value ? (findGuide(selectedCommandId.value) ?? null) : null,
)

const permissionName = computed(() =>
    currentCommand.value ? PERMISSION_NAMES[currentCommand.value.permissionLevel] : '',
)

const currentPresets = computed<UserPreset[]>(() => {
    if (!selectedCommandId.value) return []
    return userPresets.value.filter((p) => p.commandId === selectedCommandId.value)
})

const isChainCommand = computed(() => currentCommand.value?.isChainCommand === true)

// ============= 全局预设列表 =============
const allPresets = computed<UserPreset[]>(() =>
    [...userPresets.value].sort((a, b) => b.createdAt - a.createdAt),
)

// ============= 生命周期 =============
onMounted(() => {
    const manifest = loadManifest()
    versionOptions.value = manifest.versions.map((v) => ({
        value: v.id,
        label: v.label,
    }))
    selectedVersion.value = manifest.default
    initializeRegistry(manifest.default)
    userPresets.value = getUserPresets()
})

watch(selectedVersion, (versionId) => {
    if (!versionId) return
    switchVersion(versionId)
    selectedCommandId.value = ''
    values.value = {}
    executeChainOutput.value = ''
    result.value = ''
})

watch(selectedCommandId, () => {
    userPresets.value = getUserPresets()
})

// ============= 工具函数 =============
function showToast(message: string) {
    toast.value = message
    setTimeout(() => {
        toast.value = ''
    }, 1800)
}

function onCommandChange(id: unknown) {
    selectedCommandId.value = String(id)
    copied.value = false
    executeChainOutput.value = ''
    if (!selectedCommandId.value) {
        values.value = {}
        result.value = ''
        errors.value = []
        return
    }
    values.value = getDefaultValues(selectedCommandId.value)
    result.value = ''
    errors.value = []
}

function generate() {
    if (!selectedCommandId.value || !currentCommand.value) return
    copied.value = false

    // 链式命令（execute）单独处理
    if (isChainCommand.value) {
        const output = executeChainOutput.value.trim()
        if (!output) {
            errors.value = ['请先构建子命令链']
            result.value = ''
            return
        }
        errors.value = []
        result.value = `/${selectedCommandId.value} ${output}`
        return
    }

    const validationErrors = validateCommand(selectedCommandId.value, values.value)
    if (validationErrors.length) {
        errors.value = validationErrors
        result.value = ''
        return
    }
    errors.value = []
    try {
        result.value = buildCommand(selectedCommandId.value, values.value)
    } catch (e) {
        errors.value = [e instanceof Error ? e.message : 'Unknown error']
        result.value = ''
    }
}

async function copy() {
    if (!result.value) return
    try {
        await navigator.clipboard.writeText(result.value)
        copied.value = true
        setTimeout(() => {
            copied.value = false
        }, 1600)
    } catch {
        // 剪贴板不可用时静默失败
    }
}

function saveCurrentAsPreset() {
    if (!selectedCommandId.value || !currentCommand.value) return
    const name = presetNameInput.value.trim()
    if (!name) {
        showToast('请输入预设名称')
        return
    }

    if (isChainCommand.value) {
        // 链式命令：保存完整的结构化状态
        const state = executeChainRef.value?.getState()
        if (!state) {
            showToast('链式构建器未就绪')
            return
        }
        saveUserPreset(
            name,
            selectedCommandId.value,
            getCurrentVersion(),
            {},
            {
                executeState: state,
                chainString: executeChainOutput.value,
            },
        )
    } else {
        // 普通命令
        const snapshot: Record<string, string | number | boolean> = {}
        for (const arg of currentCommand.value.arguments) {
            snapshot[arg.name] = values.value[arg.name] ?? ''
        }
        saveUserPreset(name, selectedCommandId.value, getCurrentVersion(), snapshot)
    }

    userPresets.value = getUserPresets()

    presetNameInput.value = ''
    showPresetInput.value = false
    showToast(`已保存预设「${name}」`)
}

function loadPreset(preset: UserPreset) {
    if (!currentCommand.value) return

    if (isChainCommand.value) {
        // 优先使用结构化状态
        if (preset.executeState) {
            executeChainRef.value?.setState(preset.executeState)
        } else if (preset.chainString) {
            // 兼容旧预设：只有字符串
            executeChainOutput.value = preset.chainString
        }
    } else {
        values.value = applyPresetToValues(preset, currentCommand.value)
    }
    showToast(`已加载预设「${preset.name}」`)
}

function removePreset(id: string) {
    const preset = userPresets.value.find((p) => p.id === id)
    if (!preset) return

    deleteUserPreset(id)
    userPresets.value = getUserPresets()

    showToast(`已删除预设「${preset.name}」`)
}

/** 从全局预设面板跳转：切换版本 → 切换命令 → 加载数据 → 关闭面板 */
async function jumpToPreset(preset: UserPreset) {
    // 1. 切换版本（如果不同）
    if (selectedVersion.value !== preset.versionId) {
        selectedVersion.value = preset.versionId
        switchVersion(preset.versionId)
    }

    // 2. 清空筛选，保证目标命令在下拉列表里可见
    filterCategory.value = ''
    filterPermission.value = ''

    // 3. 切换命令
    selectedCommandId.value = preset.commandId
    const cmd = findCommand(preset.commandId)
    if (!cmd) {
        showToast(`预设「${preset.name}」的命令 ${preset.commandId} 在当前版本不存在`)
        showGlobalPresets.value = false
        return
    }

    // 4. 加载预设值（等 DOM 更新后 ref 才能拿到）
    await nextTick()
    if (cmd.isChainCommand) {
        if (preset.executeState) {
            executeChainRef.value?.setState(preset.executeState)
        } else if (preset.chainString) {
            executeChainOutput.value = preset.chainString
        }
    } else {
        values.value = applyPresetToValues(preset, cmd)
    }
    result.value = ''
    errors.value = []

    // 5. 关闭面板
    showGlobalPresets.value = false
    showToast(`已加载预设「${preset.name}」`)
}

function presetCategoryLabel(commandId: string): string {
    const cmd = findCommand(commandId)
    if (!cmd) return commandId
    return `${CATEGORY_LABELS[cmd.category]} · /${cmd.name}`
}
</script>

<template>
    <div class="command-generator">
        <!-- ========== 标题栏 ========== -->
        <header class="page-header">
            <span class="header-badge"><BlocksIcon /></span>
            <div class="header-titles">
                <h1>{{ formatMessage(messages.title) }}</h1>
                <p>{{ formatMessage(messages.subtitle) }}</p>
            </div>
            <button
                type="button"
                class="global-presets-button"
                @click="showGlobalPresets = !showGlobalPresets"
            >
                <BookmarkIcon />
                {{ formatMessage(messages.globalPresets) }}
                <span class="badge">{{ allPresets.length }}</span>
            </button>
            <span v-if="currentCommand" class="permission-chip">{{ permissionName }}</span>
        </header>

        <!-- ========== 全局预设面板 ========== -->
        <Transition name="slide-fade">
            <section v-if="showGlobalPresets" class="global-presets-panel">
                <header class="global-presets-head">
                    <span class="global-presets-title">
                        {{ formatMessage(messages.globalPresets) }}
                    </span>
                    <span class="global-presets-hint">
                        {{ formatMessage(messages.globalPresetsHint) }}
                    </span>
                    <button type="button" class="close-btn" @click="showGlobalPresets = false">
                        <XIcon />
                    </button>
                </header>

                <p v-if="!allPresets.length" class="global-presets-empty">
                    {{ formatMessage(messages.globalPresetsEmpty) }}
                </p>

                <div v-else class="global-presets-list">
                    <button
                        v-for="preset in allPresets"
                        :key="preset.id"
                        type="button"
                        class="global-preset-item"
                        @click="jumpToPreset(preset)"
                    >
                        <div class="global-preset-main">
                            <span class="global-preset-name">{{ preset.name }}</span>
                            <span class="global-preset-meta">
                                {{ presetCategoryLabel(preset.commandId) }}
                            </span>
                        </div>
                        <div class="global-preset-side">
                            <span class="global-preset-version">
                                {{ versionLabelMap[preset.versionId] ?? preset.versionId }}
                            </span>
                            <span
                                class="global-preset-delete"
                                @click.stop="removePreset(preset.id)"
                            >
                                <XIcon />
                            </span>
                        </div>
                    </button>
                </div>
            </section>
        </Transition>

        <!-- ========== 版本选择 ========== -->
        <section class="panel">
            <label class="field-label">{{ formatMessage(messages.selectVersion) }}</label>
            <Combobox
                :model-value="selectedVersion"
                :options="versionOptions"
                @update:model-value="selectedVersion = String($event)"
            />
        </section>

        <!-- ========== 筛选器 ========== -->
        <section class="panel">
            <div class="filter-row">
                <div class="filter-item">
                    <label class="field-label">
                        {{ formatMessage(messages.filterCategory) }}
                    </label>
                    <Combobox
                        :model-value="filterCategory"
                        :options="categoryOptions"
                        @update:model-value="filterCategory = String($event)"
                    />
                </div>
                <div class="filter-item">
                    <label class="field-label">
                        {{ formatMessage(messages.filterPermission) }}
                    </label>
                    <Combobox
                        :model-value="filterPermission"
                        :options="permissionOptions"
                        @update:model-value="filterPermission = String($event)"
                    />
                </div>
            </div>
        </section>

        <!-- ========== 命令选择 ========== -->
        <section class="panel">
            <label class="field-label">{{ formatMessage(messages.selectCommand) }}</label>
            <Combobox
                :model-value="selectedCommandId"
                :options="commandOptions"
                searchable
                @update:model-value="onCommandChange"
            >
                <template #selected="{ label }">
                    <span v-if="label" class="truncate">{{ label }}</span>
                    <span v-else class="placeholder">
                        {{ formatMessage(messages.selectPlaceholder) }}
                    </span>
                </template>
                <template #option="{ item, isSelected }">
                    <div class="option-row">
                        <span class="option-category">{{ item.category }}</span>
                        <div class="option-text">
                            <strong :class="isSelected ? 'text-brand' : ''">
                                /{{ item.label }}
                            </strong>
                            <span>{{ item.description }}</span>
                        </div>
                    </div>
                </template>
            </Combobox>
        </section>

        <!-- ========== 汉化指导 ========== -->
        <section v-if="currentGuide" class="guide-card">
            <div class="guide-head">
                <SparklesIcon />
                <span>{{ formatMessage(messages.guideTitle) }}</span>
            </div>
            <p class="guide-summary">{{ currentGuide.summary }}</p>
            <div class="guide-block">
                <span class="guide-label">{{ formatMessage(messages.guideExamples) }}</span>
                <div class="example-list">
                    <div v-for="ex in currentGuide.examples" :key="ex.command" class="example-row">
                        <code class="example-code">{{ ex.command }}</code>
                        <span class="example-desc">{{ ex.explanation }}</span>
                    </div>
                </div>
            </div>
            <div v-if="currentGuide.tips?.length" class="guide-block">
                <span class="guide-label">{{ formatMessage(messages.guideTips) }}</span>
                <ul class="tip-list">
                    <li v-for="tip in currentGuide.tips" :key="tip">{{ tip }}</li>
                </ul>
            </div>
        </section>

        <!-- ========== 参数区 ========== -->
        <section v-if="currentCommand" class="parameters-section">
            <h2 class="section-title">{{ formatMessage(messages.parametersTitle) }}</h2>

            <!-- 链式命令（execute）：走专用构建器 -->
            <CommandGeneratorExecuteChain
                v-if="isChainCommand && currentCommand.chainCategories"
                ref="executeChainRef"
                :categories="currentCommand.chainCategories"
                v-model="executeChainOutput"
            />

            <!-- 普通命令：参数表单 + 预设 -->
            <template v-else>
                <div class="preset-toolbar">
                    <Button type="outlined" size="2xs" @click="showPresetInput = !showPresetInput">
                        <BookmarkIcon />
                        {{ formatMessage(messages.savePreset) }}
                    </Button>
                    <div v-if="showPresetInput" class="preset-input-row">
                        <input
                            v-model="presetNameInput"
                            class="preset-name-input"
                            :placeholder="formatMessage(messages.presetName)"
                            @keydown.enter="saveCurrentAsPreset"
                        />
                        <Button
                            type="colored"
                            color="brand"
                            size="2xs"
                            @click="saveCurrentAsPreset"
                        >
                            OK
                        </Button>
                    </div>
                </div>

                <div v-if="currentPresets.length" class="user-presets">
                    <span class="user-presets-label">
                        {{ formatMessage(messages.myPresets) }}
                    </span>
                    <div class="user-preset-list">
                        <button
                            v-for="preset in currentPresets"
                            :key="preset.id"
                            type="button"
                            class="user-preset-chip"
                            @click="loadPreset(preset)"
                        >
                            {{ preset.name }}
                            <span class="preset-delete" @click.stop="removePreset(preset.id)">
                                <XIcon />
                            </span>
                        </button>
                    </div>
                </div>

                <div v-if="currentCommand.arguments.length" class="arguments-grid">
                    <CommandGeneratorArgument
                        v-for="arg in currentCommand.arguments"
                        :key="arg.name"
                        v-model="values[arg.name]"
                        :arg="arg"
                    />
                </div>
                <p v-else class="empty-hint">
                    {{ formatMessage(messages.parametersEmpty) }}
                </p>
            </template>

            <!-- 链式命令的预设工具栏（放在链式构建器下方） -->
            <template v-if="isChainCommand">
                <div class="preset-toolbar">
                    <Button type="outlined" size="2xs" @click="showPresetInput = !showPresetInput">
                        <BookmarkIcon />
                        {{ formatMessage(messages.savePreset) }}
                    </Button>
                    <div v-if="showPresetInput" class="preset-input-row">
                        <input
                            v-model="presetNameInput"
                            class="preset-name-input"
                            :placeholder="formatMessage(messages.presetName)"
                            @keydown.enter="saveCurrentAsPreset"
                        />
                        <Button
                            type="colored"
                            color="brand"
                            size="2xs"
                            @click="saveCurrentAsPreset"
                        >
                            OK
                        </Button>
                    </div>
                </div>

                <div v-if="currentPresets.length" class="user-presets">
                    <span class="user-presets-label">
                        {{ formatMessage(messages.myPresets) }}
                    </span>
                    <div class="user-preset-list">
                        <button
                            v-for="preset in currentPresets"
                            :key="preset.id"
                            type="button"
                            class="user-preset-chip"
                            @click="loadPreset(preset)"
                        >
                            {{ preset.name }}
                            <span class="preset-delete" @click.stop="removePreset(preset.id)">
                                <XIcon />
                            </span>
                        </button>
                    </div>
                </div>
            </template>
        </section>

        <div v-if="errors.length" class="error-panel">
            <XIcon />
            <ul>
                <li v-for="(err, i) in errors" :key="i">{{ err }}</li>
            </ul>
        </div>

        <div v-if="currentCommand" class="action-bar">
            <Button type="colored" color="brand" @click="generate">
                <SparklesIcon />
                {{ formatMessage(messages.generate) }}
            </Button>
        </div>

        <section v-if="result" class="result-card">
            <div class="result-head">
                <span class="result-badge"><CheckCircleIcon /></span>
                <span class="result-title">{{ formatMessage(messages.result) }}</span>
                <Button type="outlined" size="2xs" @click="copy">
                    {{ copied ? formatMessage(messages.copied) : formatMessage(messages.copy) }}
                </Button>
            </div>
            <pre class="result-code">{{ result }}</pre>
        </section>

        <Transition name="toast-fade">
            <div v-if="toast" class="toast">{{ toast }}</div>
        </Transition>
    </div>
</template>

<style scoped>
/* ========== 布局 ========== */
.command-generator {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;
    max-width: 64rem;
    margin: 0 auto;
    padding: 1rem 1.25rem 3rem;
}

/* ========== 标题栏 ========== */
.page-header {
    display: flex;
    align-items: center;
    gap: 0.85rem;
    padding: 0.25rem 0;
}

.header-badge {
    display: inline-flex;
    width: 2.75rem;
    height: 2.75rem;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-md);
    background: var(--surface-4);
    color: var(--color-brand);
}

.header-badge :deep(svg) {
    width: 1.35rem;
    height: 1.35rem;
}

.header-titles {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 0.2rem;
}

.header-titles h1 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 800;
    color: var(--color-text-primary);
}

.header-titles p {
    margin: 0;
    font-size: 0.75rem;
    color: var(--color-text-tertiary);
}

.permission-chip {
    flex: 0 0 auto;
    border-radius: 999px;
    background: var(--surface-4);
    border: 1px solid var(--color-divider);
    padding: 0.25rem 0.75rem;
    font-size: 0.7rem;
    font-weight: 700;
    color: var(--color-brand);
}

/* ========== 全局预设按钮 ========== */
.global-presets-button {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-3);
    padding: 0.45rem 0.85rem;
    color: var(--color-text-primary);
    font-size: 0.75rem;
    font-weight: 700;
    cursor: pointer;
    transition: background 0.15s ease;
}

.global-presets-button:hover {
    background: var(--surface-4);
}

.global-presets-button :deep(svg) {
    width: 0.9rem;
    height: 0.9rem;
}

.global-presets-button .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.25rem;
    height: 1.25rem;
    padding: 0 0.35rem;
    border-radius: 999px;
    background: var(--color-brand);
    color: #fff;
    font-size: 0.65rem;
    font-weight: 800;
}

/* ========== 全局预设面板 ========== */
.global-presets-panel {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    border: 1px solid var(--color-brand);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 1rem 1.1rem;
}

.global-presets-head {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    flex-wrap: wrap;
}

.global-presets-title {
    font-size: 0.85rem;
    font-weight: 800;
    color: var(--color-text-primary);
}

.global-presets-hint {
    flex: 1;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.close-btn {
    display: inline-flex;
    width: 1.75rem;
    height: 1.75rem;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-tertiary);
    cursor: pointer;
}

.close-btn:hover {
    background: var(--surface-4);
    color: var(--color-text-primary);
}

.close-btn :deep(svg) {
    width: 0.85rem;
    height: 0.85rem;
}

.global-presets-empty {
    margin: 0;
    padding: 0.85rem 0;
    font-size: 0.74rem;
    color: var(--color-text-tertiary);
    text-align: center;
}

.global-presets-list {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    max-height: 22rem;
    overflow-y: auto;
}

.global-preset-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    padding: 0.65rem 0.85rem;
    text-align: left;
    cursor: pointer;
    transition:
        border-color 0.15s ease,
        background 0.15s ease;
}

.global-preset-item:hover {
    border-color: var(--color-brand);
    background: var(--surface-4);
}

.global-preset-main {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 0.15rem;
}

.global-preset-name {
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--color-text-primary);
}

.global-preset-meta {
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}

.global-preset-side {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 0 0 auto;
}

.global-preset-version {
    border-radius: 999px;
    background: var(--surface-4);
    padding: 0.2rem 0.55rem;
    font-size: 0.65rem;
    font-weight: 700;
    color: var(--color-text-tertiary);
}

.global-preset-delete {
    display: inline-flex;
    width: 1.5rem;
    height: 1.5rem;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background: transparent;
    color: var(--color-text-tertiary);
    transition: color 0.15s ease;
}

.global-preset-delete:hover {
    color: var(--color-red);
}

.global-preset-delete :deep(svg) {
    width: 0.7rem;
    height: 0.7rem;
}

/* ========== 面板 ========== */
.panel {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 1rem 1.1rem;
}

.field-label {
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--color-text-primary);
}

.placeholder {
    color: var(--color-text-tertiary);
}

/* ========== 筛选器 ========== */
.filter-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.85rem;
}

.filter-item {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    min-width: 0;
}

/* ========== 选项行 ========== */
.option-row {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 0.65rem;
}

.option-category {
    flex: 0 0 auto;
    border-radius: 999px;
    background: var(--surface-4);
    padding: 0.18rem 0.55rem;
    font-size: 0.63rem;
    font-weight: 700;
    color: var(--color-text-tertiary);
}

.option-text {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.12rem;
}

.option-text strong {
    font-size: 0.84rem;
    font-weight: 700;
    color: var(--color-text-primary);
}

.option-text span {
    overflow: hidden;
    font-size: 0.7rem;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-tertiary);
}

/* ========== 汉化指导 ========== */
.guide-card {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    border: 1px solid var(--color-divider);
    border-left: 3px solid var(--color-brand);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 1rem 1.1rem;
}

.guide-head {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.72rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-brand);
}

.guide-head :deep(svg) {
    width: 0.9rem;
    height: 0.9rem;
}

.guide-summary {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.6;
    color: var(--color-text-secondary);
}

.guide-block {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
}

.guide-label {
    font-size: 0.66rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-tertiary);
}

.example-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
}

.example-row {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    border-radius: var(--radius-md);
    background: var(--surface-2);
    padding: 0.55rem 0.7rem;
}

.example-code {
    font-family: var(--mono-font, monospace);
    font-size: 0.74rem;
    color: var(--color-brand);
    word-break: break-all;
}

.example-desc {
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.tip-list {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.72rem;
    line-height: 1.55;
    color: var(--color-text-tertiary);
}

/* ========== 参数区 ========== */
.parameters-section {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
}

.section-title {
    margin: 0;
    font-size: 0.72rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-tertiary);
}

.arguments-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    gap: 0.8rem;
}

.empty-hint {
    margin: 0;
    border: 1px dashed var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-3);
    padding: 0.85rem 1rem;
    font-size: 0.74rem;
    color: var(--color-text-tertiary);
}

/* ========== 预设工具 ========== */
.preset-toolbar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
}

.preset-input-row {
    display: flex;
    gap: 0.4rem;
    flex: 1;
    min-width: 14rem;
}

.preset-name-input {
    flex: 1;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
    background: var(--surface-1);
    padding: 0.35rem 0.6rem;
    color: var(--color-text-primary);
    font-size: 0.74rem;
    outline: none;
}

.preset-name-input:focus {
    border-color: var(--color-brand);
}

.user-presets {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    background: var(--surface-3);
    padding: 0.7rem 0.85rem;
}

.user-presets-label {
    font-size: 0.66rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-tertiary);
}

.user-preset-list {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
}

.user-preset-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    border: 1px solid var(--color-brand);
    border-radius: 999px;
    background: var(--surface-1);
    padding: 0.28rem 0.7rem;
    color: var(--color-brand);
    font-size: 0.7rem;
    cursor: pointer;
    transition: background 0.15s ease;
}

.user-preset-chip:hover {
    background: var(--surface-4);
}

.preset-delete {
    display: inline-flex;
    width: 1rem;
    height: 1rem;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background: var(--surface-4);
    color: var(--color-text-tertiary);
    transition: color 0.15s ease;
}

.preset-delete:hover {
    color: var(--color-red);
}

.preset-delete :deep(svg) {
    width: 0.6rem;
    height: 0.6rem;
}

/* ========== 错误 / 操作 / 结果 ========== */
.error-panel {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    border: 1px solid var(--color-red);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 0.8rem 0.95rem;
    color: var(--color-red);
    font-size: 0.76rem;
}

.error-panel :deep(svg) {
    width: 0.95rem;
    height: 0.95rem;
    flex: 0 0 auto;
    margin-top: 0.15rem;
}

.error-panel ul {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    margin: 0;
    padding: 0;
    list-style: none;
}

.action-bar {
    display: flex;
    justify-content: center;
    gap: 0.6rem;
    padding: 0.5rem 0;
}

.result-card {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    border: 1px solid var(--color-green);
    border-radius: var(--radius-lg);
    background: var(--surface-3);
    padding: 1rem 1.1rem;
}

.result-head {
    display: flex;
    align-items: center;
    gap: 0.55rem;
}

.result-badge {
    display: grid;
    width: 1.5rem;
    height: 1.5rem;
    place-items: center;
    border-radius: 999px;
    background: var(--surface-4);
    color: var(--color-green);
}

.result-badge :deep(svg) {
    width: 0.85rem;
    height: 0.85rem;
}

.result-title {
    flex: 1;
    font-size: 0.8rem;
    font-weight: 800;
    color: var(--color-text-primary);
}

.result-code {
    margin: 0;
    max-height: 18rem;
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--surface-1);
    border: 1px solid var(--color-divider);
    padding: 0.8rem 0.9rem;
    font-family: var(--mono-font, monospace);
    font-size: 0.8rem;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--color-text-primary);
}

/* ========== Toast ========== */
.toast {
    position: fixed;
    bottom: 2rem;
    left: 50%;
    transform: translateX(-50%);
    border: 1px solid var(--color-brand);
    border-radius: 999px;
    background: var(--surface-3);
    padding: 0.55rem 1.1rem;
    color: var(--color-brand);
    font-size: 0.75rem;
    font-weight: 700;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    z-index: 1000;
    pointer-events: none;
}

.toast-fade-enter-active,
.toast-fade-leave-active {
    transition:
        opacity 0.2s ease,
        transform 0.2s ease;
}

.toast-fade-enter-from,
.toast-fade-leave-to {
    opacity: 0;
    transform: translateX(-50%) translateY(0.5rem);
}

.slide-fade-enter-active,
.slide-fade-leave-active {
    transition:
        opacity 0.2s ease,
        transform 0.2s ease;
}

.slide-fade-enter-from,
.slide-fade-leave-to {
    opacity: 0;
    transform: translateY(-0.5rem);
}
</style>
