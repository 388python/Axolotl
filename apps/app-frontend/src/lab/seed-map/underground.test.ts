import assert from 'node:assert/strict'
import test from 'node:test'

import { decodeUndergroundVolume, nextUndergroundRegion, undergroundOrigin } from './underground.ts'
import { buildUndergroundMesh, undergroundTexture } from './underground-mesh.ts'

const textureUvs: Record<string, [number, number, number, number]> = Object.fromEntries(
    ['stone', 'deepslate', 'netherrack', 'water_still', 'lava_still', 'diamond_ore'].map((id) => [
        `minecraft:block/${id}`,
        [0, 0, 1, 1],
    ]),
)

test('X-Ray hides all solid rock except faces bounding cavities and omits artificial region cut walls', () => {
    const blocks = new Uint8Array(5 * 5 * 5).fill(1)
    const request = { blocks, width: 5, height: 5, truncated: false, textureUvs }
    assert.equal(buildUndergroundMesh(request).faces, 0)
    blocks[(2 * 5 + 2) * 5 + 2] = 0
    const mesh = buildUndergroundMesh(request)
    assert.equal(mesh.faces, 6)
    assert.equal(mesh.layers.terrain.positions.length, 6 * 6 * 3)
    assert.equal(mesh.layers.ores.positions.length, 0)
    for (let index = 0; index < mesh.layers.terrain.positions.length; index += 9) {
        const p = mesh.layers.terrain.positions.slice(index, index + 9)
        const a = [p[3] - p[0], p[4] - p[1], p[5] - p[2]]
        const b = [p[6] - p[0], p[7] - p[1], p[8] - p[2]]
        const normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
        assert.deepEqual(
            normal.map((value) => value || 0),
            Array.from(mesh.layers.terrain.normals.slice(index, index + 3)),
        )
    }
})

test('minerals remain independent of invisible host rock and fluids have separate textured surfaces', () => {
    const blocks = new Uint8Array(5 * 5 * 5).fill(1)
    blocks[(2 * 5 + 2) * 5 + 2] = 16
    const mesh = buildUndergroundMesh({ blocks, width: 5, height: 5, truncated: false, textureUvs })
    assert.equal(mesh.layers.ores.positions.length, 6 * 6 * 3)
    assert.equal(mesh.layers.terrain.positions.length, 0)
    blocks.fill(0)
    blocks[(2 * 5 + 2) * 5 + 2] = 4
    blocks[(3 * 5 + 2) * 5 + 2] = 4
    const fluids = buildUndergroundMesh({
        blocks,
        width: 5,
        height: 5,
        truncated: false,
        textureUvs,
    })
    assert.equal(fluids.faces, 10)
    assert.equal(fluids.layers.water.positions.length, 10 * 6 * 3)
    assert.equal(fluids.layers.lava.positions.length, 0)
    assert.equal(undergroundTexture(80, 0), 'minecraft:block/deepslate_diamond_ore')
    assert.equal(undergroundTexture(25, 1), 'minecraft:block/ancient_debris_top')
    assert.equal(undergroundTexture(2, 1), 'minecraft:block/deepslate_top')
})

test('underground origins honor negative coordinates, legacy heights and world boundaries', () => {
    const query = {
        seed: '1',
        edition: 'java' as const,
        version: '26.3',
        dimension: 'overworld' as const,
        x: -1,
        y: -63,
        z: -17,
    }
    assert.deepEqual(undergroundOrigin(query), [-48, -64, -64])
    assert.deepEqual(
        undergroundOrigin({ ...query, x: 30_000_000, y: 319, z: -30_000_000 }),
        [29_999_936, 256, -30_000_000],
    )
    assert.equal(undergroundOrigin({ ...query, version: '1.17.1' })[1], 0)
    assert.equal(undergroundOrigin({ ...query, dimension: 'nether', y: 127 })[1], 64)
})

test('volume decoder rejects incompatible data and retains truncation status', () => {
    assert.throws(() => decodeUndergroundVolume(new ArrayBuffer(16)))
    const buffer = new ArrayBuffer(16 + 64 ** 3)
    const header = new DataView(buffer)
    ;[1, 1, 64, 64].forEach((value, index) => header.setUint32(index * 4, value, true))
    assert.equal(decodeUndergroundVolume(buffer).truncated, true)
    assert.equal(decodeUndergroundVolume(buffer).blocks.length, 64 ** 3)
})

test('walking preloads an overlapping region and stops requesting at clamped world boundaries', () => {
    const query = {
        seed: '1',
        edition: 'java' as const,
        version: '26.3',
        dimension: 'overworld' as const,
        x: 32,
        y: 32,
        z: 32,
    }
    assert.equal(nextUndergroundRegion(query, [0, 0, 0], [32, 32, 32]), undefined)
    const next = nextUndergroundRegion(query, [0, 0, 0], [48, 32, 32])
    assert.ok(next)
    assert.deepEqual(undergroundOrigin(next), [16, 0, 0])
    assert.equal(nextUndergroundRegion(query, [16, 0, 0], [48, 32, 32]), undefined)
    const vertical = nextUndergroundRegion(query, [0, 0, 0], [32, 49, 32])
    assert.ok(vertical)
    assert.deepEqual(undergroundOrigin(vertical), [0, 16, 0])
    assert.equal(
        nextUndergroundRegion(
            query,
            [-30_000_000, -64, -30_000_000],
            [-30_000_001, -65, -30_000_001],
        ),
        undefined,
    )
})
