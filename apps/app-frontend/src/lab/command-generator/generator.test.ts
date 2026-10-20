import assert from 'node:assert/strict'
import { before, describe, it } from 'node:test'

import { getAllCommands, initializeRegistry } from './commands/registry.ts'
import { buildCommand, getDefaultValues, validateCommand } from './generator.ts'

before(() => {
    initializeRegistry()
})

describe('buildCommand', () => {
    it('generates a basic give command', () => {
        assert.equal(
            buildCommand('give', {
                target: '@s',
                item: 'minecraft:diamond',
                count: 64,
            }),
            '/give @s minecraft:diamond 64',
        )
    })

    it('generates a summon command with NBT', () => {
        assert.equal(
            buildCommand('summon', {
                entity: 'minecraft:axolotl',
                pos: '~ ~ ~',
                nbt: '{Variant:0}',
            }),
            '/summon minecraft:axolotl ~ ~ ~ {Variant:0}',
        )
    })

    it('generates a command with no arguments', () => {
        assert.equal(buildCommand('seed', {}), '/seed')
        assert.equal(buildCommand('reload', {}), '/reload')
    })

    it('appends boolean arguments only when true', () => {
        assert.equal(buildCommand('list', { uuids: true }), '/list uuids')
        assert.equal(buildCommand('list', { uuids: false }), '/list')
    })

    it('automatically quotes message arguments', () => {
        assert.equal(buildCommand('say', { message: 'Hello World' }), '/say "Hello World"')
    })

    it('skips empty optional arguments', () => {
        assert.equal(
            buildCommand('gamemode', { mode: 'creative', target: '' }),
            '/gamemode creative',
        )
    })

    it('throws an error when a required argument is missing', () => {
        assert.throws(() => buildCommand('gamemode', {}))
    })

    it('throws an error for an unknown command', () => {
        assert.throws(() => buildCommand('nonexistent', {}), /Unknown command/)
    })
})

describe('validateCommand', () => {
    it('returns an empty error array for valid arguments', () => {
        assert.equal(validateCommand('gamemode', { mode: 'creative' }).length, 0)
    })

    it('returns errors when required fields are missing', () => {
        assert.ok(validateCommand('give', {}).length > 0)
    })

    // Note: `gamemode` uses a custom "gamemode" type, so the enum validation in `generator.ts` does not apply.
    // If enum validation is needed for custom types, the `validateCommand` logic should be updated.

    it('returns errors for non-integer values passed to integer arguments', () => {
        assert.ok(
            validateCommand('give', {
                target: '@s',
                item: 'minecraft:stone',
                count: 1.5,
            }).length > 0,
        )
    })
})

describe('getDefaultValues', () => {
    it('returns preset default values', () => {
        assert.equal(getDefaultValues('give').count, 1)
    })

    it('returns an empty object for an unknown command', () => {
        assert.deepEqual(getDefaultValues('nonexistent'), {})
    })
})

describe('registry completeness', () => {
    it('all commands have unique ids', () => {
        const commands = getAllCommands()
        const ids = commands.map((c) => c.id)
        assert.equal(new Set(ids).size, ids.length)
    })

    it('all commands have descriptions and categories', () => {
        for (const cmd of getAllCommands()) {
            assert.ok(cmd.descriptionKey)
            assert.ok(cmd.category)
        }
    })
})
