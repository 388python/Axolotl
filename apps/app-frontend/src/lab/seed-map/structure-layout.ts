import type { SeedMapStructureDetails, SeedMapStructurePiece } from './advanced.ts'

export const SEED_MAP_LAYOUT_MAX_SCALE = 2

export function seedMapPieceBounds(piece: SeedMapStructurePiece) {
    return { min: piece.min ?? piece.position, max: piece.max ?? piece.position }
}

export function seedMapLayoutPieces(details: SeedMapStructureDetails | null) {
    return (
        details?.pieces.filter(
            (piece) =>
                piece.positionKnown &&
                piece.min &&
                piece.max &&
                ['x', 'z'].every((axis) => {
                    const key = axis as 'x' | 'z'
                    return (
                        Number.isFinite(piece.min![key]) &&
                        Number.isFinite(piece.max![key]) &&
                        piece.max![key] >= piece.min![key]
                    )
                }),
        ) ?? []
    )
}

export function seedMapVolumePieces(details: SeedMapStructureDetails | null) {
    return seedMapLayoutPieces(details).filter(
        (piece) =>
            piece.min &&
            piece.max &&
            ['x', 'y', 'z'].every((axis) => {
                const key = axis as 'x' | 'y' | 'z'
                return (
                    Number.isFinite(piece.min![key]) &&
                    Number.isFinite(piece.max![key]) &&
                    piece.max![key] >= piece.min![key]
                )
            }),
    )
}

export function seedMapStructureCapabilities(details: SeedMapStructureDetails) {
    return {
        variant: details.capabilities?.variant ?? !!details.variant,
        projection: seedMapLayoutPieces(details).length > 0,
        volume: seedMapVolumePieces(details).length > 0,
        containers: details.capabilities?.containers ?? details.containers.length > 0,
        complete:
            seedMapLayoutPieces(details).length > 0 &&
            seedMapLayoutPieces(details).length === details.pieces.length &&
            !details.truncated &&
            !details.limits.some((limit) =>
                ['partial_layout', 'layout_unavailable', 'piece_positions_unavailable'].includes(
                    limit,
                ),
            ),
        absoluteHeight: false,
        blocks: false,
    }
}

export function drawSeedMapLayout(
    context: CanvasRenderingContext2D,
    pieces: SeedMapStructurePiece[],
    scale: number,
    toScreen: (x: number, z: number) => { x: number; y: number },
    selected?: number,
) {
    if (scale > SEED_MAP_LAYOUT_MAX_SCALE) return
    context.save()
    const palette = seedMapLayoutPalette(context.canvas)
    for (const piece of pieces) {
        const { min, max } = seedMapPieceBounds(piece)
        const point = toScreen(min.x, min.z)
        const width = (max.x - min.x + 1) / scale
        const height = (max.z - min.z + 1) / scale
        if (
            point.x + width < 0 ||
            point.y + height < 0 ||
            point.x > context.canvas.width ||
            point.y > context.canvas.height
        )
            continue
        context.fillStyle = piece.id === selected ? palette.selected : palette.normal
        context.globalAlpha = piece.id === selected ? 0.35 : 0.2
        context.fillRect(point.x, point.y, width, height)
        context.globalAlpha = 1
        context.strokeStyle = piece.id === selected ? palette.selected : palette.normal
        context.lineWidth = piece.id === selected ? 2 : 1
        context.strokeRect(point.x, point.y, width, height)
    }
    context.restore()
}

export function seedMapLayoutPalette(element: Element) {
    const style = getComputedStyle(element)
    return {
        normal: style.getPropertyValue('--color-blue').trim(),
        selected: style.getPropertyValue('--color-orange').trim(),
    }
}

export function hitSeedMapLayout(
    pieces: SeedMapStructurePiece[],
    x: number,
    z: number,
    selected?: number,
) {
    const hits = pieces.filter((piece) => {
        const { min, max } = seedMapPieceBounds(piece)
        return x >= min.x && x < max.x + 1 && z >= min.z && z < max.z + 1
    })
    hits.sort((a, b) => {
        const size = (piece: SeedMapStructurePiece) => {
            const { min, max } = seedMapPieceBounds(piece)
            return (max.x - min.x + 1) * (max.z - min.z + 1)
        }
        return size(a) - size(b) || a.id - b.id
    })
    const index = hits.findIndex((piece) => piece.id === selected)
    return hits[(index + 1) % hits.length]
}
