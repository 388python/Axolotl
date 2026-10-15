<script setup lang="ts">
import { ChevronDownIcon, ChevronUpIcon, PlusIcon, XIcon } from '@modrinth/assets'
import { Combobox, type ComboboxOption } from '@modrinth/ui'
import { computed, ref } from 'vue'

import type { NbtTagNode, NbtTagType } from '@/lab/command-generator/nbt-tree.ts'

const props = defineProps<{
    node: NbtTagNode
    depth: number
}>()

const emit = defineEmits<{
    update: [id: string, field: string, value: string]
    remove: [id: string]
    'add-child': [id: string]
}>()

const expanded = ref(true)

const isContainer = computed(() => props.node.type === 'compound' || props.node.type === 'list')

const typeOptions: ComboboxOption[] = [
    { value: 'string', label: '字符串' },
    { value: 'int', label: '整数' },
    { value: 'short', label: '短整' },
    { value: 'byte', label: '字节' },
    { value: 'float', label: '浮点' },
    { value: 'double', label: '双精度' },
    { value: 'compound', label: '复合' },
    { value: 'list', label: '列表' },
]

function onTypeChange(value: unknown) {
    emit('update', props.node.id, 'type', String(value) as NbtTagType)
}
</script>

<template>
    <div class="nbt-node">
        <div class="nbt-row" :style="{ paddingLeft: `${depth * 16}px` }">
            <button
                v-if="isContainer"
                type="button"
                class="nbt-toggle"
                :aria-expanded="expanded"
                @click="expanded = !expanded"
            >
                <ChevronUpIcon v-if="expanded" />
                <ChevronDownIcon v-else />
            </button>
            <span v-else class="nbt-toggle-spacer" />

            <input
                class="nbt-input nbt-name"
                :value="node.name"
                @input="emit('update', node.id, 'name', ($event.target as HTMLInputElement).value)"
            />

            <div class="nbt-type">
                <Combobox
                    :model-value="node.type"
                    :options="typeOptions"
                    @update:model-value="onTypeChange"
                />
            </div>

            <input
                v-if="!isContainer"
                class="nbt-input nbt-value"
                :value="String(node.value)"
                @input="emit('update', node.id, 'value', ($event.target as HTMLInputElement).value)"
            />
            <span v-else class="nbt-child-count"> {{ node.children?.length ?? 0 }} 个子项 </span>

            <button
                v-if="isContainer"
                type="button"
                class="nbt-action nbt-add"
                @click="emit('add-child', node.id)"
            >
                <PlusIcon />
            </button>
            <button type="button" class="nbt-action nbt-remove" @click="emit('remove', node.id)">
                <XIcon />
            </button>
        </div>

        <template v-if="isContainer && expanded">
            <CommandGeneratorNbtNode
                v-for="child in node.children"
                :key="child.id"
                :node="child"
                :depth="depth + 1"
                @update="(id, f, v) => emit('update', id, f, v)"
                @remove="(id) => emit('remove', id)"
                @add-child="(id) => emit('add-child', id)"
            />
        </template>
    </div>
</template>

<style scoped>
.nbt-node {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.nbt-row {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 0.35rem;
}

.nbt-toggle,
.nbt-action {
    display: inline-flex;
    width: 1.35rem;
    height: 1.35rem;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-tertiary);
    cursor: pointer;
    transition:
        background 0.15s ease,
        color 0.15s ease;
}

.nbt-toggle:hover,
.nbt-action:hover {
    background: var(--surface-4);
    color: var(--color-text-primary);
}

.nbt-toggle :deep(svg),
.nbt-action :deep(svg) {
    width: 0.75rem;
    height: 0.75rem;
}

.nbt-toggle-spacer {
    width: 1.35rem;
    flex: 0 0 auto;
}

.nbt-input {
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    padding: 0.25rem 0.5rem;
    color: var(--color-text-primary);
    font-size: 0.72rem;
    outline: none;
    transition: border-color 0.15s ease;
}

.nbt-input:focus {
    border-color: var(--color-brand);
}

.nbt-name {
    width: 7rem;
    flex: 0 0 auto;
}

.nbt-type {
    width: 8.5rem;
    flex: 0 0 auto;
}

.nbt-value {
    min-width: 0;
    flex: 1;
}

.nbt-child-count {
    flex: 1;
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}

.nbt-add {
    color: var(--color-brand);
}

.nbt-remove {
    color: var(--color-red);
}
</style>
