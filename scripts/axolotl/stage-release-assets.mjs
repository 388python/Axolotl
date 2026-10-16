import fs from 'node:fs'
import path from 'node:path'
import { parseArgs } from 'node:util'

import { files, root } from './desktop-build-utils.mjs'

const { values } = parseArgs({
    options: {
        input: { type: 'string' },
        output: { type: 'string' },
        platform: { type: 'string' },
    },
})
if (!values.input || !values.output || !['linux', 'macos'].includes(values.platform)) {
    throw new Error('Expected --input, --output and --platform linux|macos')
}
const destination = path.resolve(root, values.output)
fs.mkdirSync(destination, { recursive: true })
const suffixes =
    values.platform === 'linux'
        ? ['.deb', '.rpm', '.AppImage', '.AppImage.tar.gz']
        : ['.dmg', '.app.tar.gz']
const selected = files(path.resolve(root, values.input)).filter((file) =>
    suffixes.some((suffix) => file.endsWith(suffix)),
)
for (const suffix of suffixes) {
    if (selected.filter((file) => file.endsWith(suffix)).length !== 1) {
        throw new Error(`Expected exactly one ${suffix} artifact`)
    }
}
for (const file of selected) {
    const name =
        values.platform === 'macos' && file.endsWith('.app.tar.gz')
            ? 'Axolotl.Launcher_universal.app.tar.gz'
            : path.basename(file).replaceAll(' ', '.')
    fs.copyFileSync(file, path.join(destination, name))
    if (fs.existsSync(`${file}.sig`))
        fs.copyFileSync(`${file}.sig`, path.join(destination, `${name}.sig`))
    if (
        (file.endsWith('.AppImage.tar.gz') || file.endsWith('.app.tar.gz')) &&
        !fs.existsSync(`${file}.sig`)
    ) {
        throw new Error(`Missing updater signature for ${name}`)
    }
}
