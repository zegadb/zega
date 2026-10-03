import { describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FAKE_SERVER, startCli, workdir } from './cli_helpers'
import { remove } from './helpers'
import { directoryAssets, embeddedAssets } from '../src/handlers/explorer_assets'
import { filesModule, inventoryOf, listFiles, type Inventory } from '../src/handlers/explorer_inventory'

const EXPLORER = path.resolve(import.meta.dir, '..', 'src', 'explorer')
const DIST = path.join(EXPLORER, 'page')
const inventory = JSON.parse(fs.readFileSync(path.join(EXPLORER, 'inventory.json'), 'utf8')) as Inventory

describe('the vendored explorer is what its inventory says', () => {
    test('built from a commit of zegadb/zega, and every file is byte for byte what was recorded (inventory.json and files.ts regenerate to themselves)', () => {
        expect(inventory.source).toBe('zegadb/zega')
        expect(inventory.commit).toMatch(/^[0-9a-f]{40}$/)
        expect(inventoryOf(DIST, inventory.commit)).toEqual(inventory)
        expect(fs.readFileSync(path.join(EXPLORER, 'files.ts'), 'utf8')).toBe(filesModule(listFiles(DIST)))
    })

    test('it is the CLI build: no link to the website, no samples, no demo components', () => {
        const files = Object.keys(inventory.files)
        expect(files).toContain('index.html')
        expect(files).toContain('pkg/zega_wasm_bg.wasm')
        expect(files.filter((file) => /^(samples|components)\//.test(file))).toEqual([])
        const html = fs.readFileSync(path.join(DIST, 'index.html'), 'utf8')
        expect(html).not.toContain('zega.dev')
        expect(html).not.toMatch(/<a\b/)
        for (const sample of ['flights', 'westeros', 'calgary', 'tickets', 'cities', 'seed']) expect(html).not.toContain(`btn-${sample}`)
    })
})

describe('serving the vendored files', () => {
    test('every file is served with a type; a name that is not a file, a path out of the directory and an object property are not', async () => {
        const assets = await embeddedAssets()
        const wasm = assets.get('pkg/zega_wasm_bg.wasm')
        expect(wasm?.type).toBe('application/wasm')
        expect(wasm?.size).toBeGreaterThan(1_000_000)
        expect(assets.get('index.html')?.type).toBe('text/html; charset=utf-8')
        expect(assets.get('repl.js')?.type).toBe('text/javascript; charset=utf-8')
        for (const name of Object.keys(inventory.files)) expect(assets.get(name)?.size).toBeGreaterThan(0)
        for (const name of ['nope.js', '../inventory.json', 'constructor', '__proto__', 'samples/flights.zql']) expect(assets.get(name)).toBeNull()
    })

    test('a directory of files is served the same way and cannot be left', () => {
        const assets = directoryAssets(path.resolve(import.meta.dir, 'fixtures', 'explorer-assets'))
        expect(assets.get('index.html')?.type).toBe('text/html; charset=utf-8')
        expect(assets.get('../fake-zega-server.ts')).toBeNull()
        expect(assets.get('nothing')).toBeNull()
    })

    test.skipIf(process.platform === 'win32')('`zega explorer` with nothing else given serves the vendored page', async () => {
        const dir = workdir()
        try {
            const cli = startCli(['explorer', '--port', '0', '--server', FAKE_SERVER, '--data', path.join(dir, 'data')], { cwd: dir, home: path.join(dir, 'home') })
            await cli.until('Ctrl-C to stop', 'stderr')
            const url = cli.stdout().split('\n')[0]!
            const html = await (await fetch(url)).text()
            expect(html).toBe(fs.readFileSync(path.join(DIST, 'index.html'), 'utf8'))
            const wasm = await fetch(`${url}/pkg/zega_wasm_bg.wasm`)
            expect(wasm.headers.get('content-type')).toBe('application/wasm')
            expect(Buffer.from(await wasm.arrayBuffer()).byteLength).toBe(fs.statSync(path.join(DIST, 'pkg', 'zega_wasm_bg.wasm')).size)
            cli.signal('SIGTERM')
            expect(await cli.exited).toBe(0)
        } finally {
            remove(dir)
        }
    })
})
