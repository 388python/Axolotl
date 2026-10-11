<script setup lang="ts">
import { Avatar, Button, defineMessages, NewModal, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, ref } from 'vue'

import type { InstallJobSnapshot } from '@/helpers/install'
import { resourceOperationError } from '@/helpers/resource-operation-logic'
import {
    type ResourceDependencyNode,
    resourceGet,
    type ResourceOperationIntent,
    type ResourceOperationPlan,
    type ResourcePlanDecision,
    resourcePrepareEntryOperation,
    resourcePrepareOperation,
    resourceQueuePlan,
    resourceRevisePlan,
    type SourceReleaseRef,
} from '@/helpers/resources'

const { formatMessage } = useVIntl()
const messages = defineMessages({
    header: { id: 'app.resources.operation.header', defaultMessage: 'Review content changes' },
    loading: {
        id: 'app.resources.operation.loading',
        defaultMessage: 'Resolving releases and dependencies…',
    },
    artifact: {
        id: 'app.resources.operation.artifact',
        defaultMessage: '{source} · release {release} · {file}',
    },
    required: { id: 'app.resources.operation.required', defaultMessage: 'Required dependencies' },
    optional: { id: 'app.resources.operation.optional', defaultMessage: 'Available dependencies' },
    satisfied: { id: 'app.resources.operation.satisfied', defaultMessage: 'Already satisfied' },
    changes: {
        id: 'app.resources.operation.changes',
        defaultMessage:
            '{count, plural, one {# file will change} other {# files will change}} in {instance}.',
    },
    add: { id: 'app.resources.operation.add', defaultMessage: 'Add {name}' },
    remove: { id: 'app.resources.operation.remove', defaultMessage: 'Remove {name}' },
    requiredBy: {
        id: 'app.resources.operation.required-by',
        defaultMessage: 'Required by {names}',
    },
    exception: {
        id: 'app.resources.operation.exception',
        defaultMessage: 'Allow this release despite its game version or loader assessment',
    },
    assessment: {
        id: 'app.resources.operation.assessment',
        defaultMessage: 'Game version: {game}; loader: {loader}',
    },
    state: {
        id: 'app.resources.operation.compatibility',
        defaultMessage:
            '{state, select, compatible {Compatible} incompatible {Incompatible} other {Unknown}}',
    },
    retry: { id: 'app.resources.operation.retry', defaultMessage: 'Retry' },
    refresh: { id: 'app.resources.operation.refresh', defaultMessage: 'Refresh this preview' },
    cancel: { id: 'app.resources.operation.cancel', defaultMessage: 'Cancel' },
    apply: { id: 'app.resources.operation.apply', defaultMessage: 'Queue changes' },
    empty: { id: 'app.resources.operation.empty', defaultMessage: 'No file changes are needed.' },
})

type Input = {
    instanceId: string
    instanceName: string
    entryIds: string[]
    intent: ResourceOperationIntent
    release?: SourceReleaseRef | null
    installDependencies: boolean
}
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const plan = ref<ResourceOperationPlan | null>(null)
const input = ref<Input | null>(null)
const busy = ref(false)
const queueing = ref(false)
const queueFailed = ref(false)
const error = ref<string | null>(null)
const names = ref<Record<string, { title: string; iconUrl: string | null }>>({})
let controller: AbortController | null = null
let generation = 0
let resolveShow: ((job: InstallJobSnapshot | null) => void) | null = null
let retryAction: (() => Promise<void>) | null = null

const active = computed(
    () => plan.value?.nodes.filter((node) => node.state !== 'optionalCandidate') ?? [],
)
const candidates = computed(
    () => plan.value?.nodes.filter((node) => node.state === 'optionalCandidate') ?? [],
)
const canApply = computed(
    () =>
        !busy.value &&
        (plan.value?.actions.length ?? 0) > 0 &&
        !plan.value?.issues.some((issue) => issue.blocking),
)
function name(node: ResourceDependencyNode) {
    return names.value[node.id]?.title ?? node.artifact?.fileName ?? ''
}
function requiredBy(id: string) {
    return (plan.value?.edges ?? [])
        .filter((edge) => edge.child === id && edge.kind === 'required')
        .map((edge) => plan.value?.nodes.find((node) => node.id === edge.parent))
        .filter((node): node is ResourceDependencyNode => !!node)
        .map(name)
        .join(', ')
}
async function enrich(value: ResourceOperationPlan, signal: AbortSignal, current: number) {
    await Promise.allSettled(
        value.nodes.map(async (node) => {
            const resource = await resourceGet(node.resource, undefined, undefined, signal)
            if (current !== generation || signal.aborted) return
            const project =
                resource.projects.find(
                    (project) =>
                        project.project.source === node.artifact?.reference.release.project.source,
                ) ?? resource.projects[0]
            if (project)
                names.value = {
                    ...names.value,
                    [node.id]: { title: project.title, iconUrl: project.iconUrl },
                }
        }),
    )
}
async function run(action: (signal: AbortSignal) => Promise<ResourceOperationPlan>) {
    const current = generation
    controller?.abort()
    controller = new AbortController()
    const signal = controller.signal
    busy.value = true
    error.value = null
    retryAction = () => run(action)
    try {
        const value = await action(signal)
        if (current !== generation || signal.aborted) return
        plan.value = value
        void enrich(value, signal, current)
    } catch (failure) {
        if (current === generation && !signal.aborted) error.value = resourceOperationError(failure)
    } finally {
        if (current === generation) busy.value = false
    }
}
async function revise(decision: ResourcePlanDecision) {
    const value = plan.value
    if (!value || busy.value) return
    await run((signal) => resourceRevisePlan(value.id, value.revision, [decision], signal))
}
function hide() {
    generation++
    controller?.abort()
    const resolve = resolveShow
    resolveShow = null
    resolve?.(null)
}
function show(value: Input): Promise<InstallJobSnapshot | null> {
    hide()
    input.value = value
    plan.value = null
    names.value = {}
    error.value = null
    queueFailed.value = false
    modal.value?.show()
    const result = new Promise<InstallJobSnapshot | null>((resolve) => {
        resolveShow = resolve
    })
    void run((signal) =>
        resourcePrepareEntryOperation(
            value.instanceId,
            value.intent,
            value.entryIds,
            value.release ?? null,
            value.installDependencies,
            signal,
        ),
    )
    return result
}
async function apply() {
    if (!plan.value || !canApply.value) return
    busy.value = true
    queueing.value = true
    error.value = null
    try {
        const job = await resourceQueuePlan(plan.value.id, plan.value.revision)
        const resolve = resolveShow
        resolveShow = null
        resolve?.(job)
        modal.value?.hide()
    } catch (failure) {
        error.value = resourceOperationError(failure)
        retryAction = apply
        queueFailed.value = true
    } finally {
        busy.value = false
        queueing.value = false
    }
}
async function refresh() {
    const current = plan.value
    const value = input.value
    if (!current || !value || busy.value) return
    queueFailed.value = false
    await run(async (signal) => {
        const prepared = await resourcePrepareOperation(
            value.instanceId,
            current.intent,
            current.selections,
            value.installDependencies,
            signal,
        )
        if (!current.exceptions.length) return prepared
        return resourceRevisePlan(
            prepared.id,
            prepared.revision,
            current.exceptions.map((exception) => ({ kind: 'acceptCompatibility', exception })),
            signal,
        )
    })
}
function accept(node: ResourceDependencyNode) {
    if (!node.artifact) return
    const dimensions = (['gameVersion', 'loader'] as const).filter(
        (dimension) => node.compatibility[dimension] !== 'compatible',
    )
    void revise({
        kind: 'acceptCompatibility',
        exception: { selectionId: node.id, release: node.artifact.reference.release, dimensions },
    })
}
onBeforeUnmount(hide)
defineExpose({ show })
</script>

<template>
    <NewModal
        ref="modal"
        :header="formatMessage(messages.header)"
        :on-hide="hide"
        :disable-close="queueing"
        scrollable
        width="min(42rem, 90vw)"
    >
        <div class="flex flex-col gap-3">
            <p v-if="busy" class="m-0">{{ formatMessage(messages.loading) }}</p>
            <p v-if="plan" class="m-0">
                {{
                    formatMessage(messages.changes, {
                        count: plan.actions.length,
                        instance: input?.instanceName ?? '',
                    })
                }}
            </p>
            <div v-if="error" class="flex flex-col gap-2" role="alert">
                <p class="m-0">{{ error }}</p>
                <Button v-if="retryAction" :disabled="busy" @click="retryAction?.()">{{
                    formatMessage(messages.retry)
                }}</Button>
                <Button v-if="queueFailed" :disabled="busy" @click="refresh">{{
                    formatMessage(messages.refresh)
                }}</Button>
            </div>
            <div
                v-for="node in active"
                :key="node.id"
                class="flex flex-col gap-2 rounded-lg border border-surface-4 bg-surface-2 p-3"
            >
                <div class="flex items-center gap-2">
                    <Avatar
                        :src="names[node.id]?.iconUrl ?? undefined"
                        :alt="name(node)"
                        size="32px"
                    />
                    <span class="grow font-semibold">{{ name(node) }}</span>
                    <Button
                        v-if="plan?.activeRoots.includes(node.id)"
                        size="sm"
                        :disabled="busy"
                        :aria-label="formatMessage(messages.remove, { name: name(node) })"
                        @click="revise({ kind: 'removeRoot', selectionId: node.id })"
                        >×</Button
                    >
                </div>
                <p v-if="node.artifact" class="m-0 break-all text-sm">
                    {{
                        formatMessage(messages.artifact, {
                            source: node.artifact.reference.release.project.source,
                            release: node.artifact.reference.release.releaseId,
                            file: node.artifact.fileName ?? '',
                        })
                    }}
                </p>
                <p v-if="node.scope.kind === 'world'" class="m-0 break-all text-sm">
                    {{ node.scope.relativePath }}
                </p>
                <p v-if="node.state === 'satisfied'" class="m-0 text-sm">
                    {{ formatMessage(messages.satisfied) }}
                </p>
                <p v-if="requiredBy(node.id)" class="m-0 text-sm">
                    {{ formatMessage(messages.requiredBy, { names: requiredBy(node.id) }) }}
                </p>
                <p
                    v-for="action in plan?.actions.filter((action) => action.nodeId === node.id)"
                    :key="action.entryId"
                    class="m-0 break-all text-sm"
                >
                    {{ action.targetRelativePath }}
                </p>
                <template
                    v-if="
                        node.compatibility.gameVersion !== 'compatible' ||
                        node.compatibility.loader !== 'compatible'
                    "
                >
                    <p class="m-0 text-sm">
                        {{
                            formatMessage(messages.assessment, {
                                game: formatMessage(messages.state, {
                                    state: node.compatibility.gameVersion,
                                }),
                                loader: formatMessage(messages.state, {
                                    state: node.compatibility.loader,
                                }),
                            })
                        }}
                    </p>
                    <Button
                        v-if="
                            plan?.issues.some(
                                (issue) =>
                                    issue.selectionId === node.id &&
                                    ['compatibilityUnknown', 'compatibilityMismatch'].includes(
                                        issue.code,
                                    ),
                            )
                        "
                        :disabled="busy"
                        @click="accept(node)"
                        >{{ formatMessage(messages.exception) }}</Button
                    >
                </template>
            </div>
            <template v-if="candidates.length">
                <h3 class="m-0">{{ formatMessage(messages.optional) }}</h3>
                <div
                    v-for="node in candidates"
                    :key="node.id"
                    class="flex items-center gap-2 rounded-lg border border-surface-4 p-3"
                >
                    <Avatar
                        :src="names[node.id]?.iconUrl ?? undefined"
                        :alt="name(node)"
                        size="32px"
                    />
                    <span class="grow">{{ name(node) }}</span>
                    <Button
                        size="sm"
                        :disabled="busy"
                        :aria-label="formatMessage(messages.add, { name: name(node) })"
                        @click="revise({ kind: 'addOptional', nodeId: node.id })"
                        >＋</Button
                    >
                </div>
            </template>
            <p v-for="(issue, index) in plan?.issues" :key="index" class="m-0 text-sm">
                {{ issue.message }}
            </p>
            <p v-if="plan && !busy && !plan.actions.length" class="m-0">
                {{ formatMessage(messages.empty) }}
            </p>
        </div>
        <template #actions>
            <Button :disabled="queueing" @click="modal?.hide()">{{
                formatMessage(messages.cancel)
            }}</Button>
            <Button color="brand" :disabled="!canApply" @click="apply">{{
                formatMessage(messages.apply)
            }}</Button>
        </template>
    </NewModal>
</template>
