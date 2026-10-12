import type { ExportEmbeddedFailureChoice, ExportResolutionSession } from '@/helpers/resources'

export function pendingFailures(
    session: ExportResolutionSession | null,
    embedded: string[],
): ExportEmbeddedFailureChoice[] {
    return (
        session?.files.flatMap((file) =>
            file.resolutions
                .filter(
                    (resolution) =>
                        resolution.kind === 'failed' &&
                        embedded.includes(file.snapshot.relativePath),
                )
                .map((resolution) => ({ snapshot: file.snapshot, source: resolution.source })),
        ) ?? []
    )
}
export function analysisReady(
    session: ExportResolutionSession | null,
    selected: string[],
    embedded: string[],
): boolean {
    if (
        !session ||
        session.state === 'refreshing' ||
        session.state === 'canceled' ||
        session.files.length !== selected.length
    )
        return false
    const paths = new Set(selected)
    return session.files.every(
        (file) =>
            paths.has(file.snapshot.relativePath) &&
            !file.localError &&
            file.snapshot.integrity.hashes.length >= 3 &&
            (!file.snapshot.identifiable ||
                ['modrinth', 'curseforge'].every((source) =>
                    file.resolutions.some((resolution) => resolution.source === source),
                )) &&
            (!file.resolutions.some((r) => r.kind === 'failed') ||
                embedded.includes(file.snapshot.relativePath)),
    )
}
