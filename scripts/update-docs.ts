/**
 * Vendor the documentation `zega mcp` searches from a zegadb/zega checkout:
 *
 *     bun run docs:update ../zega
 *
 * It refuses a checkout with uncommitted changes (the recorded commit must be what was copied), copies
 * `docs/*.md` into `src/docs/pages/` (replacing what was there, file by file), and writes `src/docs/inventory.json`
 * (the commit and the SHA-256 of every file) and `src/docs/pages.ts`. Commit all of it together; `bun test` fails
 * when they disagree.
 */
import fs from 'node:fs'
import path from 'node:path'

import { pagesModule } from '../src/handlers/docs_inventory'
import { inventoryOf, listFiles } from '../src/handlers/explorer_inventory'

const ROOT = path.resolve(import.meta.dir, '..')
const TARGET = path.join(ROOT, 'src', 'docs')
const PAGES = path.join(TARGET, 'pages')

const from = process.argv[2]
if (from === undefined) {
    console.error('usage: bun run docs:update <path to a zegadb/zega checkout>')
    process.exit(1)
}
const zega = path.resolve(from)
const run = (command: string[]): string => {
    const result = Bun.spawnSync(command, { cwd: zega, stdout: 'pipe', stderr: 'pipe' })
    if (result.exitCode !== 0) {
        console.error(`${command.join(' ')} failed in ${zega}:\n${result.stderr.toString()}${result.stdout.toString()}`)
        process.exit(1)
    }
    return result.stdout.toString().trim()
}
if (run(['git', 'status', '--porcelain', '--untracked-files=no']) !== '') {
    console.error(`${zega} has uncommitted changes: commit them first (the commit recorded here must be what was copied)`)
    process.exit(1)
}
const commit = run(['git', 'rev-parse', 'HEAD'])
const source = path.join(zega, 'docs')
const wanted = fs.readdirSync(source).filter((name) => name.endsWith('.md'))
if (wanted.length === 0) {
    console.error(`${source} has no .md files`)
    process.exit(1)
}
fs.mkdirSync(PAGES, { recursive: true })
for (const old of listFiles(PAGES)) if (!wanted.includes(old)) fs.unlinkSync(path.join(PAGES, old))
for (const name of wanted) fs.copyFileSync(path.join(source, name), path.join(PAGES, name))

fs.writeFileSync(path.join(TARGET, 'inventory.json'), JSON.stringify(inventoryOf(PAGES, commit), null, 2) + '\n')
fs.writeFileSync(path.join(TARGET, 'pages.ts'), pagesModule(listFiles(PAGES)))
console.log(`vendored ${wanted.length} pages from zegadb/zega ${commit}`)
