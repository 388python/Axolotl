<script setup lang="ts">
import { TrashIcon } from '@modrinth/assets'
import { Button, NewModal, useVIntl } from '@modrinth/ui'
import { nextTick, ref } from 'vue'

import { purgeLauncherCache } from '@/helpers/cache.js'
import { useTheming } from '@/store/theme'

import { storageMessages } from './storageMessages'

const props = withDefaults(defineProps<{ compact?: boolean }>(), { compact: false })
const emit = defineEmits<{ cleared: [] }>()
const { formatMessage } = useVIntl()
const themeStore = useTheming()
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const busy = ref(false)
const closing = ref(false)
const failed = ref(false)
const done = ref(false)

async function show(event: MouseEvent) {
    if (busy.value || closing.value) return
    failed.value = false
    done.value = false
    if (themeStore.getFeatureFlag('skip_non_essential_warnings')) {
        await clear()
        if (!failed.value) return
    }
    modal.value?.show(event)
}
async function clear() {
    if (busy.value || closing.value) return
    busy.value = true
    failed.value = false
    try {
        await purgeLauncherCache()
        closing.value = true
        busy.value = false
        await nextTick()
        await modal.value?.hide()
        done.value = true
        emit('cleared')
    } catch (error) {
        failed.value = true
        console.warn('[storage] Cache cleanup failed', error)
    } finally {
        busy.value = false
        closing.value = false
    }
}
</script>

<template>
    <div class="flex items-center gap-3" @click.stop @keydown.stop>
        <span
            v-if="done && !props.compact"
            class="text-sm text-[var(--color-text-tertiary)]"
            role="status"
            >{{ formatMessage(storageMessages.cacheCleared) }}</span
        >
        <Button
            :size="props.compact ? '2xs' : 'md'"
            :loading="busy"
            :disabled="closing"
            @click="show"
            ><TrashIcon />{{
                formatMessage(
                    done && props.compact
                        ? storageMessages.cacheCleared
                        : storageMessages.clearCache,
                )
            }}</Button
        >
    </div>
    <NewModal
        ref="modal"
        :header="formatMessage(storageMessages.clearCache)"
        :disable-close="busy"
        width="min(36rem, 90vw)"
    >
        <p class="m-0">{{ formatMessage(storageMessages.clearCacheHelp) }}</p>
        <p v-if="failed" role="alert" class="text-sm text-red">
            {{ formatMessage(storageMessages.cacheClearFailed) }}
        </p>
        <template #actions>
            <div class="flex flex-wrap justify-end gap-3">
                <Button :disabled="busy || closing" @click="modal?.hide()">{{
                    formatMessage(storageMessages.cancel)
                }}</Button>
                <Button
                    type="colored"
                    color="red"
                    :loading="busy"
                    :disabled="closing"
                    @click="clear"
                    >{{
                        formatMessage(
                            busy ? storageMessages.clearingCache : storageMessages.clearCache,
                        )
                    }}</Button
                >
            </div>
        </template>
    </NewModal>
</template>
