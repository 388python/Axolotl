<script setup lang="ts">
import {
    Button,
    Combobox,
    defineMessages,
    injectNotificationManager,
    Toggle,
    useVIntl,
} from '@modrinth/ui'
import { computed, inject, onMounted, ref } from 'vue'

import { type AIState, getAIState, sharedAIState } from '@/helpers/ai'
import {
    get_crash_analysis_ai_settings,
    get_log_share_settings,
    update_crash_analysis_ai_settings,
    update_log_share_settings,
} from '@/helpers/logs'
import type { AnalysisSettings } from '@/logagent/types'

import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'

type LogShareSettings = {
    share_provider: 'logshare' | 'mclogs'
    ai_source: string
    auto_upload: boolean
    multi_file: boolean
    no_storage: boolean
    show_progress: boolean
}

const { formatMessage } = useVIntl()
const replayOnboarding = inject<(mode: 'logagent') => Promise<void>>('replayOnboarding')
const { handleError } = injectNotificationManager()
const settings = ref<LogShareSettings>({
    share_provider: 'logshare',
    ai_source: 'logshare',
    auto_upload: true,
    multi_file: true,
    no_storage: false,
    show_progress: true,
})
const aiSettings = ref<AnalysisSettings>({
    enabled: false,
    provider_id: '',
    model_id: '',
    ai_source: 'logshare',
})
const loading = ref(true)
const saving = ref(false)

const messages = defineMessages({
    tour: { id: 'app.logagent.settings.tour', defaultMessage: 'About analysis sources' },
    supportedModels: {
        id: 'app.logagent.settings.supported-models',
        defaultMessage:
            'Choose a model that supports tool calling. Protocol support does not guarantee that every model accepts tools. Copilot Auto requires selecting an explicit model.',
    },
    title: { id: 'app.log-share.settings.title', defaultMessage: 'Log sharing & AI analysis' },
    description: {
        id: 'app.log-share.settings.description',
        defaultMessage:
            'Choose local LogAgent or cloud LogShare for crash analysis. Your log sharing service is configured separately.',
    },
    shareProvider: {
        id: 'app.log-share.settings.share-provider',
        defaultMessage: 'Log sharing service',
    },
    shareProviderDescription: {
        id: 'app.log-share.settings.share-provider-description',
        defaultMessage: 'Where the diagnostic link is uploaded when you share it.',
    },
    aiSource: { id: 'app.log-share.settings.ai-source', defaultMessage: 'AI analysis source' },
    aiSourceLogShare: {
        id: 'app.log-share.settings.ai-source-logshare',
        defaultMessage: 'Cloud LogShare',
    },
    aiSourceCustom: {
        id: 'app.log-share.settings.ai-source-custom',
        defaultMessage: 'Local LogAgent',
    },
    aiSourceDescription: {
        id: 'app.log-share.settings.ai-source-description',
        defaultMessage:
            'Local LogAgent runs in the launcher and sends relevant log excerpts to your configured model provider. Cloud LogShare sends logs to LogShare. This is the default; you can change the source for each analysis.',
    },
    enabled: {
        id: 'app.crash-analysis.ai.settings.enabled',
        defaultMessage: 'Enable local LogAgent',
    },
    enabledDescription: {
        id: 'app.crash-analysis.ai.settings.enabled-description',
        defaultMessage: 'Local rule-based crash analysis remains available without AI.',
    },
    provider: { id: 'app.crash-analysis.ai.settings.provider', defaultMessage: 'AI provider' },
    model: { id: 'app.crash-analysis.ai.settings.model', defaultMessage: 'Text model' },
    noProviders: {
        id: 'app.crash-analysis.ai.settings.no-providers',
        defaultMessage:
            'Enable an AI provider and a tool-capable model in Settings > AI before configuring local LogAgent.',
    },
    autoUpload: {
        id: 'app.log-share.settings.auto-upload',
        defaultMessage: 'Upload to get a structured summary automatically',
    },
    autoUploadDescription: {
        id: 'app.log-share.settings.auto-upload-description',
        defaultMessage: 'Only effective in LogShare AI mode.',
    },
    multiFile: { id: 'app.log-share.settings.multi-file', defaultMessage: 'Multi-file upload' },
    multiFileDescription: {
        id: 'app.log-share.settings.multi-file-description',
        defaultMessage: 'Upload latest.log, crash reports, hs_err and launcher logs together.',
    },
    noStorage: {
        id: 'app.log-share.settings.no-storage',
        defaultMessage: 'Analyze without storing logs',
    },
    noStorageDescription: {
        id: 'app.log-share.settings.no-storage-description',
        defaultMessage:
            'Submit content directly for analysis and skip the stored upload. Disables the automatic summary.',
    },
    showProgress: {
        id: 'app.log-share.settings.show-progress',
        defaultMessage: 'Show AI analysis progress',
    },
    showProgressDescription: {
        id: 'app.log-share.settings.show-progress-description',
        defaultMessage:
            'Show readable investigation steps and keep their history after analysis. Diagnostic text appears as it is generated when the provider supports streaming.',
    },
})

const emptyState: AIState = { settings: { enabled: false }, catalog_source: '', providers: [] }
const aiState = computed(() => sharedAIState.value ?? emptyState)
const providers = computed(() =>
    aiState.value.providers.filter(
        (provider) => provider.enabled && provider.models.some((model) => model.enabled),
    ),
)
const providerOptions = computed(() =>
    providers.value.map((provider) => ({
        value: provider.provider_id,
        label: provider.custom_name || provider.provider_id,
    })),
)
const modelOptions = computed(
    () =>
        providers.value
            .find((provider) => provider.provider_id === aiSettings.value.provider_id)
            ?.models.filter((model) => model.enabled)
            .map((model) => ({ value: model.id, label: model.name || model.id })) ?? [],
)

const isCustom = computed(() => aiSettings.value.ai_source === 'custom')
const noStorageEnabled = computed(() => aiSettings.value.ai_source === 'logshare')

const shareProviderOptions = [
    { value: 'logshare', label: 'LogShare' },
    { value: 'mclogs', label: 'mclo.gs' },
]
const aiSourceOptions = computed(() => [
    { value: 'logshare', label: formatMessage(messages.aiSourceLogShare) },
    { value: 'custom', label: formatMessage(messages.aiSourceCustom) },
])

async function saveShare(next: LogShareSettings): Promise<void> {
    saving.value = true
    try {
        await update_log_share_settings(next)
        settings.value = next
    } catch (error) {
        handleError(error)
    } finally {
        saving.value = false
    }
}

async function saveAI(next: AnalysisSettings): Promise<void> {
    saving.value = true
    try {
        await update_crash_analysis_ai_settings(next)
        aiSettings.value = next
        settings.value = { ...settings.value, ai_source: next.ai_source }
    } catch (error) {
        handleError(error)
    } finally {
        saving.value = false
    }
}

function updateShare(patch: Partial<LogShareSettings>): void {
    void saveShare({ ...settings.value, ...patch })
}

function selectShareProvider(value: unknown): void {
    const provider = value === 'mclogs' ? 'mclogs' : 'logshare'
    updateShare({ share_provider: provider, ai_source: aiSettings.value.ai_source })
}

function selectAiSource(value: unknown): void {
    const source = value === 'custom' ? 'custom' : 'logshare'
    void saveAI({ ...aiSettings.value, ai_source: source })
}

function updateEnabled(enabled: boolean): void {
    void saveAI({ ...aiSettings.value, enabled })
}

function updateProvider(providerId: string): void {
    const provider = providers.value.find((item) => item.provider_id === providerId)
    const modelId = provider?.models.find((model) => model.enabled)?.id ?? ''
    void saveAI({ ...aiSettings.value, provider_id: providerId, model_id: modelId })
}

function updateModel(modelId: string): void {
    void saveAI({ ...aiSettings.value, model_id: modelId })
}

onMounted(async () => {
    try {
        const [nextShare, nextAI] = await Promise.all([
            get_log_share_settings(),
            get_crash_analysis_ai_settings(),
            getAIState().catch(handleError),
        ])
        settings.value = { ...nextShare, ai_source: nextAI.ai_source }
        aiSettings.value = nextAI
    } catch (error) {
        handleError(error)
    } finally {
        loading.value = false
    }
})
</script>

<template>
    <SettingsSection
        v-if="!loading"
        id="settings-target-log-share"
        :title="formatMessage(messages.title)"
        :description="formatMessage(messages.description)"
    >
        <SettingsRow data-onboarding-id="logagent-settings-sharing">
            <template #label>{{ formatMessage(messages.shareProvider) }}</template>
            <template #description>{{ formatMessage(messages.shareProviderDescription) }}</template>
            <template #control>
                <Combobox
                    id="log-share-provider"
                    :model-value="settings.share_provider"
                    :options="shareProviderOptions"
                    :disabled="saving"
                    @update:model-value="selectShareProvider($event)"
                />
            </template>
        </SettingsRow>
        <template #extra>
            <Button type="outlined" @click="replayOnboarding?.('logagent')">{{
                formatMessage(messages.tour)
            }}</Button>
        </template>
        <SettingsRow data-onboarding-id="logagent-settings-source">
            <template #label>{{ formatMessage(messages.aiSource) }}</template>
            <template #description>{{ formatMessage(messages.aiSourceDescription) }}</template>
            <template #control>
                <Combobox
                    id="log-share-ai-source"
                    :model-value="aiSettings.ai_source"
                    :options="aiSourceOptions"
                    :disabled="saving"
                    @update:model-value="selectAiSource($event)"
                />
            </template>
        </SettingsRow>
        <p v-if="isCustom" class="m-0 px-4 py-3 text-sm text-[var(--color-text-tertiary)]">
            {{ formatMessage(messages.supportedModels) }}
        </p>
        <template v-if="isCustom">
            <SettingsRow>
                <template #label>{{ formatMessage(messages.enabled) }}</template>
                <template #description>{{ formatMessage(messages.enabledDescription) }}</template>
                <template #control>
                    <Toggle
                        id="crash-analysis-ai-enabled"
                        :model-value="aiSettings.enabled"
                        :disabled="saving || !aiState.settings.enabled || !providers.length"
                        @update:model-value="updateEnabled(!!$event)"
                    />
                </template>
            </SettingsRow>
            <template v-if="providers.length">
                <SettingsRow>
                    <template #label>{{ formatMessage(messages.provider) }}</template>
                    <template #control>
                        <Combobox
                            id="crash-analysis-ai-provider"
                            :model-value="aiSettings.provider_id"
                            :options="providerOptions"
                            :disabled="saving"
                            @update:model-value="updateProvider(String($event))"
                        />
                    </template>
                </SettingsRow>
                <SettingsRow>
                    <template #label>{{ formatMessage(messages.model) }}</template>
                    <template #control>
                        <Combobox
                            id="crash-analysis-ai-model"
                            :model-value="aiSettings.model_id"
                            :options="modelOptions"
                            :disabled="saving || !aiSettings.provider_id"
                            @update:model-value="updateModel(String($event))"
                        />
                    </template>
                </SettingsRow>
            </template>
            <p v-else class="m-0 p-4 text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.noProviders) }}
            </p>
        </template>
        <SettingsRow>
            <template #label>{{ formatMessage(messages.autoUpload) }}</template>
            <template #description>{{ formatMessage(messages.autoUploadDescription) }}</template>
            <template #control>
                <Toggle
                    id="log-share-auto-upload"
                    :model-value="settings.auto_upload"
                    :disabled="saving || !noStorageEnabled || settings.no_storage"
                    @update:model-value="updateShare({ auto_upload: !!$event })"
                />
            </template>
        </SettingsRow>
        <SettingsRow>
            <template #label>{{ formatMessage(messages.multiFile) }}</template>
            <template #description>{{ formatMessage(messages.multiFileDescription) }}</template>
            <template #control>
                <Toggle
                    id="log-share-multi-file"
                    :model-value="settings.multi_file"
                    :disabled="saving"
                    @update:model-value="updateShare({ multi_file: !!$event })"
                />
            </template>
        </SettingsRow>
        <SettingsRow data-onboarding-id="logagent-settings-cloud">
            <template #label>{{ formatMessage(messages.noStorage) }}</template>
            <template #description>{{ formatMessage(messages.noStorageDescription) }}</template>
            <template #control>
                <Toggle
                    id="log-share-no-storage"
                    :model-value="settings.no_storage"
                    :disabled="saving || !noStorageEnabled"
                    @update:model-value="updateShare({ no_storage: !!$event })"
                />
            </template>
        </SettingsRow>
        <SettingsRow>
            <template #label>{{ formatMessage(messages.showProgress) }}</template>
            <template #description>{{ formatMessage(messages.showProgressDescription) }}</template>
            <template #control>
                <Toggle
                    id="log-share-show-progress"
                    :model-value="settings.show_progress"
                    :disabled="saving"
                    @update:model-value="updateShare({ show_progress: !!$event })"
                />
            </template>
        </SettingsRow>
    </SettingsSection>
</template>
