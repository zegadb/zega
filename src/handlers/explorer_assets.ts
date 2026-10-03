import fs from 'node:fs'
import path from 'node:path'

/** Where the explorer page's files come from. `get` is a request path without the leading slash. */
export type Assets = { get(path: string): Blob | null }

/** Types Bun does not name (or names generically). */
const TYPES: Record<string, string> = {
    '.geojson': 'application/geo+json',
    '.pmtiles': 'application/octet-stream',
    '.wasm': 'application/wasm',
    '.mjs': 'text/javascript; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.css': 'text/css; charset=utf-8',
    '.html': 'text/html; charset=utf-8',
    '.json': 'application/json',
}

function typed(file: string, name: string): Blob {
    const type = TYPES[path.extname(name).toLowerCase()]
    const blob = Bun.file(file)
    return type === undefined ? blob : blob.slice(0, blob.size, type)
}

/** The files vendored in `src/explorer/dist` (and embedded in a compiled executable). */
export async function embeddedAssets(): Promise<Assets> {
    const { FILES } = await import('@/explorer/files')
    return { get: (name) => (Object.hasOwn(FILES, name) ? typed(FILES[name]!, name) : null) }
}

/** A directory of built files (`--assets`: a `dist-cli` from a zega checkout, to try a build that is not vendored yet). */
export function directoryAssets(dir: string): Assets {
    const root = path.resolve(dir)
    return {
        get: (name) => {
            const file = path.resolve(root, name)
            if (file !== root && !file.startsWith(root + path.sep)) return null
            return fs.existsSync(file) && fs.statSync(file).isFile() ? typed(file, name) : null
        },
    }
}
