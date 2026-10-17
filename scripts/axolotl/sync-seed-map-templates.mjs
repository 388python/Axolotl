import { createHash } from 'node:crypto'
import fs from 'node:fs/promises'
import { createRequire } from 'node:module'
import { gzipSync } from 'node:zlib'

import { PNG } from 'pngjs'

import { averageTextureColor } from './seed-map-texture-colors.mjs'

const require = createRequire(new URL('../../apps/app-frontend/package.json', import.meta.url))
const { unzipSync } = require('fflate')
const { NbtFile, BlockState } = require('deepslate')
const path = process.argv[2]
if (!path)
    throw new Error(
        'Usage: node scripts/axolotl/sync-seed-map-templates.mjs <client.jar> [expected-version]',
    )
const jar = await fs.readFile(path)
const entries = unzipSync(jar, {
    filter: ({ name }) =>
        name === 'version.json' ||
        /^data\/minecraft\/structures?\/(end_city|shipwreck|igloo|pillager_outpost|ruined_portal|nether_fossils)\/\w+\.nbt$/.test(
            name,
        ) ||
        /^assets\/minecraft\/(blockstates|models)\/.+\.json$/.test(name) ||
        /^assets\/minecraft\/textures\/(block|entity)\/.+\.png(\.mcmeta)?$/.test(name),
})
const readJson = (name) => JSON.parse(new TextDecoder().decode(entries[name]))
const version = readJson('version.json')
if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(version.id) || !Number.isInteger(version.world_version))
    throw new Error('Invalid JAR version metadata')
if (process.argv[3] && version.id !== process.argv[3])
    throw new Error(`Expected ${process.argv[3]}, received ${version.id}`)
const templates = {}
const hashes = {}
const palette = new Map()
for (const [name, bytes] of Object.entries(entries).sort(([a], [b]) => a.localeCompare(b))) {
    if (!name.endsWith('.nbt')) continue
    const source = NbtFile.read(bytes).root.toSimplifiedJson()
    const sourcePalettes = source.palettes ?? [source.palette]
    if (!sourcePalettes[0]) throw new Error(`Missing template palette: ${name}`)
    const id = name.replace(/^data\/minecraft\/structures?\//, '').slice(0, -4)
    const palettes = sourcePalettes.map((entries) =>
        entries.map((state) => ({
            name: state.id ?? state.Name,
            properties: state.properties ?? state.Properties ?? {},
        })),
    )
    const blocks = source.blocks.map((block) => {
        if (!block.nbt?.final_state) return [...block.pos, block.state]
        const replacement = BlockState.parse(block.nbt.final_state)
        const state = {
            name: replacement.getName().toString(),
            properties: replacement.getProperties(),
        }
        const index = palettes[0].length
        for (const palette of palettes) palette.push(state)
        return [...block.pos, index]
    })
    for (const state of palettes.flat()) {
        if (typeof state.name !== 'string') throw new Error(`Invalid block state in ${name}`)
        palette.set(state.name, state)
    }
    templates[id] = {
        size: source.size,
        palette: palettes[0],
        ...(palettes.length > 1 ? { alternativePalettes: palettes } : {}),
        blocks,
        overwriteAir: id.startsWith('end_city/')
            ? ![
                  'second_floor_1',
                  'second_floor_2',
                  'third_floor_1',
                  'third_floor_2',
                  'second_roof',
              ].includes(id.split('/')[1])
            : !id.startsWith('pillager_outpost/'),
    }
    hashes[id] = createHash('sha256').update(bytes).digest('hex')
}
const required = [
    'base_floor',
    'base_roof',
    'bridge_end',
    'bridge_gentle_stairs',
    'bridge_piece',
    'bridge_steep_stairs',
    'fat_tower_base',
    'fat_tower_middle',
    'fat_tower_top',
    'second_floor_1',
    'second_floor_2',
    'second_roof',
    'ship',
    'third_floor_1',
    'third_floor_2',
    'third_roof',
    'tower_base',
    'tower_floor',
    'tower_piece',
    'tower_top',
]
if (required.some((name) => !templates[`end_city/${name}`]))
    throw new Error('Missing required End city templates')
const resources = { blockDefinitions: {}, blockModels: {}, defaultBlockProperties: {} }
const identifier = (id) => (id.includes(':') ? id : `minecraft:${id}`)
const pendingModels = new Set()
function collectModels(value) {
    if (Array.isArray(value)) {
        value.forEach(collectModels)
        return
    }
    if (!value || typeof value !== 'object') return
    for (const [key, item] of Object.entries(value)) {
        if (key === 'model' && typeof item === 'string') pendingModels.add(identifier(item))
        else collectModels(item)
    }
}
for (const name of [...palette.keys()].sort()) {
    const asset = `assets/minecraft/blockstates/${name.replace('minecraft:', '')}.json`
    if (!entries[asset]) continue
    const definition = readJson(asset)
    resources.blockDefinitions[name] = definition
    collectModels(definition)
}
const visited = new Set()
while (pendingModels.size) {
    const id = pendingModels.values().next().value
    pendingModels.delete(id)
    if (visited.has(id)) continue
    visited.add(id)
    const [namespace, path] = id.split(':')
    const asset = `assets/${namespace}/models/${path}.json`
    if (!entries[asset]) {
        if (path.startsWith('builtin/')) continue
        throw new Error(`Missing block model ${id}`)
    }
    const model = readJson(asset)
    if (model.textures)
        model.textures = Object.fromEntries(
            Object.entries(model.textures).map(([key, texture]) => [
                key,
                typeof texture === 'object' ? texture.sprite : texture,
            ]),
        )
    resources.blockModels[id] = model
    if (model.parent) pendingModels.add(identifier(model.parent))
}
const textureColors = {}
for (const [name, bytes] of Object.entries(entries).sort(([a], [b]) => a.localeCompare(b))) {
    if (!name.endsWith('.png')) continue
    const image = PNG.sync.read(Buffer.from(bytes))
    const metadata = entries[`${name}.mcmeta`] ? readJson(`${name}.mcmeta`) : undefined
    const color = averageTextureColor(image, metadata?.animation)
    if (color)
        textureColors[`minecraft:${name.slice('assets/minecraft/textures/'.length, -4)}`] = color
}
const root = new URL('../../apps/app-frontend/src/lab/seed-map/assets/', import.meta.url)
const output = new URL(`versions/${version.id}/`, root)
await fs.mkdir(output, { recursive: true })
const bytes = gzipSync(
    JSON.stringify({
        schemaVersion: 2,
        version: version.id,
        dataVersion: version.world_version,
        templates,
        resources,
        textureColors,
    }),
    { level: 9 },
)
await fs.writeFile(new URL('structures.json.gz', output), bytes)
await fs.writeFile(
    new URL('sources.json', output),
    JSON.stringify(
        {
            source: `Minecraft Java Edition ${version.id} client JAR`,
            copyright: 'Minecraft assets © Mojang AB / Microsoft. All rights reserved.',
            jarSha256: createHash('sha256').update(jar).digest('hex'),
            assetSha256: createHash('sha256').update(bytes).digest('hex'),
            version: version.id,
            dataVersion: version.world_version,
            templates: hashes,
            colorAlgorithm:
                'alpha-weighted linear-sRGB mean of nontransparent pixels, weighted by animation frame duration; per model face texture',
            textureCount: Object.keys(textureColors).length,
            placementRules:
                'template-v1: EndCityPiece air processors; Shipwreck pivot (4,0,15) encoded by engine origin; Igloo pivots and offsets from IglooPieces; Outpost main tower only; RuinedPortal center pivot and FRONT_BACK mirror, excluding terrain processors; NetherFossil variant. Verify rules against each new game version.',
            omitted: ['entities', 'block entity contents', 'data marker entities'],
        },
        null,
        4,
    ) + '\n',
)
const catalogUrl = new URL('catalog.json', root)
let catalog = { schemaVersion: 1, versions: {} }
try {
    catalog = JSON.parse(await fs.readFile(catalogUrl, 'utf8'))
} catch (error) {
    if (error.code !== 'ENOENT') throw error
}
catalog.versions[version.id] = {
    dataVersion: version.world_version,
    structures: Object.entries({
        'end-city': 'end_city/base_floor',
        shipwreck: 'shipwreck/rightsideup_full',
        igloo: 'igloo/top',
        outpost: 'pillager_outpost/watchtower',
        'ruined-portal': 'ruined_portal/portal_1',
        'nether-fossil': 'nether_fossils/fossil_1',
    })
        .filter(([, id]) => templates[id])
        .map(([kind]) => kind),
    templates: Object.fromEntries(
        Object.entries(templates).map(([id, template]) => [
            id,
            { size: template.size, paletteCount: template.alternativePalettes?.length ?? 1 },
        ]),
    ),
}
catalog.versions = Object.fromEntries(
    Object.entries(catalog.versions).sort(([a], [b]) => a.localeCompare(b)),
)
await fs.writeFile(catalogUrl, JSON.stringify(catalog, null, 4) + '\n')
console.log(
    `Generated ${version.id}: ${Object.keys(templates).length} templates, ${Object.keys(resources.blockModels).length} models, ${Object.keys(textureColors).length} texture colors (${bytes.length} bytes compressed)`,
)
