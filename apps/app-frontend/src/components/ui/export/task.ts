import type { InstallJobSnapshot } from '../../../helpers/install'

/** Observation errors keep the task active until its actual terminal state arrives. */
export async function observeExportTask(
    read: () => Promise<InstallJobSnapshot>,
    signal: AbortSignal,
    pause: () => Promise<void> = () => new Promise((resolve) => setTimeout(resolve, 1000)),
): Promise<InstallJobSnapshot | null> {
    while (!signal.aborted) {
        try {
            const job = await readWithTimeout(read)
            if (signal.aborted) return null
            if (['succeeded', 'failed', 'interrupted', 'canceled'].includes(job.status)) return job
        } catch {
            if (signal.aborted) return null
        }
        await pause()
    }
    return null
}

function readWithTimeout(read: () => Promise<InstallJobSnapshot>): Promise<InstallJobSnapshot> {
    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('Export task query timed out')), 10000)
        read()
            .then(resolve, reject)
            .finally(() => clearTimeout(timeout))
    })
}
