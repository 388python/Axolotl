import { invoke } from '@tauri-apps/api/core'

import type { SchematicMeshData } from '../schematic-preview/mesh-worker'
import type { SeedMapEdition } from './backend'
import type { SeedMapDimension } from './biomes'

export type UndergroundQuery = {
    seed: string
    edition: SeedMapEdition
    version: string
    dimension: SeedMapDimension
    x: number
    y: number
    z: number
}
export type UndergroundLayer = 'terrain' | 'water' | 'lava' | 'ores'
export type UndergroundVolume = {
    blocks: Uint8Array
    width: number
    height: number
    truncated: boolean
}
export type UndergroundMesh = {
    layers: Record<UndergroundLayer, SchematicMeshData>
    spawn: [number, number, number]
    faces: number
    noAir: boolean
}
export type UndergroundMeshRequest = UndergroundVolume & {
    textureUvs: Record<string, [number, number, number, number]>
}

export function undergroundOrigin(query: UndergroundQuery): [number, number, number] {
    const modern =
        Number(query.version.split('.')[0]) >= 26 || /^1\.(?:1[89]|2\d)(?:\.|$)/.test(query.version)
    const min = query.dimension === 'overworld' && modern ? -64 : 0
    const max = query.dimension === 'nether' ? 128 : modern ? 320 : 256
    const horizontal = (value: number) =>
        Math.max(-30_000_000, Math.min(29_999_936, Math.floor((value - 32) / 16) * 16))
    return [
        horizontal(query.x),
        Math.max(min, Math.min(max - 64, Math.floor((query.y - 32) / 8) * 8)),
        horizontal(query.z),
    ]
}

/** Recenter before reaching an unloaded edge, keeping an overlapping region visible. */
export function nextUndergroundRegion(
    query: UndergroundQuery,
    origin: [number, number, number],
    position: [number, number, number],
): UndergroundQuery | undefined {
    if (position.every((value, axis) => value - origin[axis] >= 16 && value - origin[axis] < 48))
        return
    const next = { ...query, x: position[0], y: position[1], z: position[2] }
    if (undergroundOrigin(next).every((value, axis) => value === origin[axis])) return
    return next
}

export function decodeUndergroundVolume(buffer: ArrayBuffer): UndergroundVolume {
    if (buffer.byteLength < 16) throw new Error('Invalid underground volume header')
    const header = new DataView(buffer)
    const width = header.getUint32(8, true)
    const height = header.getUint32(12, true)
    if (
        header.getUint32(0, true) !== 1 ||
        width !== 64 ||
        height !== 64 ||
        buffer.byteLength !== 16 + width * width * height
    ) {
        throw new Error('Unsupported underground volume format')
    }
    return {
        blocks: new Uint8Array(buffer, 16),
        width,
        height,
        truncated: Boolean(header.getUint32(4, true) & 1),
    }
}

export async function loadUndergroundVolume(query: UndergroundQuery, id: number) {
    return decodeUndergroundVolume(
        await invoke<ArrayBuffer>('plugin:seed-map|seed_map_underground_volume', {
            request: { ...query, id },
        }),
    )
}

export function cancelUndergroundVolume(id: number) {
    return invoke('plugin:seed-map|seed_map_cancel_underground', { id })
}
