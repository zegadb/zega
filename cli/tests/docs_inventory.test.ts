import { describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { PAGES } from '../src/docs/pages'

const ROOT = path.resolve(import.meta.dir, '..', 'src', 'docs')

describe('vendored docs inventory', () => {
    test('inventory, markdown files and generated page map contain exactly the same documentation, with recorded bytes', () => {
        const inventory = JSON.parse(fs.readFileSync(path.join(ROOT, 'inventory.json'), 'utf8')) as { files: Record<string, string> }
        const disk = fs.readdirSync(path.join(ROOT, 'pages')).filter((name) => name.endsWith('.md')).sort()
        expect(Object.keys(inventory.files).sort()).toEqual(disk)
        expect(Object.keys(PAGES).map((name) => `${name}.md`).sort()).toEqual(disk)
        for (const name of disk) {
            const text = fs.readFileSync(path.join(ROOT, 'pages', name), 'utf8')
            const hash = new Bun.CryptoHasher('sha256').update(text).digest('hex')
            expect(hash).toBe(inventory.files[name]!)
            expect(PAGES[name!.slice(0, -3)]!).toBe(text)
        }
    })
})
