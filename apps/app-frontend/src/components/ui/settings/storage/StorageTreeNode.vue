<script setup lang="ts">
import { computed } from 'vue'

import type { StorageNode } from './storageData'
import { sortStorageChildren } from './storageData'
import StorageDisclosure from './StorageDisclosure.vue'
import StorageTreeRow from './StorageTreeRow.vue'

defineOptions({ name: 'StorageTreeNode' })

const props = defineProps<{
    node: StorageNode
    depth: number
    parentTotal: number
}>()

const emit = defineEmits<{
    action: [node: StorageNode]
}>()

const hasChildren = computed(() => (props.node.children?.length ?? 0) > 0)
const totalSize = computed(() => props.node.size.actual + props.node.size.symlink)
const visibleChildren = computed(() => sortStorageChildren(props.node.children))
</script>

<template>
    <StorageDisclosure :expandable="hasChildren">
        <template #summary="{ open }">
            <StorageTreeRow
                :node="node"
                :depth="depth"
                :parent-total="parentTotal"
                :expanded="open"
                @action="emit('action', $event)"
                ><template #actions><slot name="actions" /></template
            ></StorageTreeRow>
        </template>
        <div>
            <StorageTreeNode
                v-for="child in visibleChildren"
                :key="child.id"
                :node="child"
                :depth="depth + 1"
                :parent-total="totalSize"
                @action="emit('action', $event)"
            />
        </div>
    </StorageDisclosure>
</template>
