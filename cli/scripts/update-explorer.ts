/**
 * Vendor the explorer page from a zegadb/zega checkout:
 *
 *     bun run explorer:update ../zega
 *
 * It builds `dist-cli/` there (`npm run build:cli` in `browser/`: the explorer without the website's
 * links and samples), refuses a checkout with uncommitted changes (the recorded commit would not be
 * what was built), copies the files into `src/explorer/page/` (replacing what was there, file by file),
 * and writes `src/explorer/inventory.json` (commit and SHA-256 of every file) and `src/explorer/files.ts`.
 * Commit all of it together; `bun test` fails when they disagree.
 */
import fs from 'node:fs'
import path from 'node:path'

import { filesModule, inventoryOf, listFiles } from '../src/handlers/explorer_inventory'

const ROOT = path.resolve(import.meta.dir, '..')
const TARGET = path.join(ROOT, 'src', 'explorer')
const DIST = path.join(TARGET, 'page')

const from = process.argv[2]
if (from === undefined) {
    console.error('usage: bun run explorer:update <path to a zegadb/zega checkout>')
    process.exit(1)
}
const zega = path.resolve(from)
const run = (command: string[], cwd: string): string => {
    const result = Bun.spawnSync(command, { cwd, stdout: 'pipe', stderr: 'pipe' })
    if (result.exitCode !== 0) {
        console.error(`${command.join(' ')} failed in ${cwd}:\n${result.stderr.toString()}${result.stdout.toString()}`)
        process.exit(1)
    }
    return result.stdout.toString().trim()
}

if (run(['git', 'status', '--porcelain', '--untracked-files=no'], zega) !== '') {
    console.error(`${zega} has uncommitted changes: commit them first (the commit recorded here must be what was built)`)
    process.exit(1)
}
const commit = run(['git', 'rev-parse', 'HEAD'], zega)
run(['node', 'scripts/build.mjs', '--cli'], path.join(zega, 'browser'))
const built = path.join(zega, 'browser', 'dist-cli')

// Replace file by file: nothing outside src/explorer/page is touched.
fs.mkdirSync(DIST, { recursive: true })
const wanted = new Set(listFiles(built))
for (const old of listFiles(DIST)) if (!wanted.has(old)) fs.unlinkSync(path.join(DIST, old))
for (const file of wanted) {
    fs.mkdirSync(path.dirname(path.join(DIST, file)), { recursive: true })
    fs.copyFileSync(path.join(built, file), path.join(DIST, file))
}
const emptyDirs = (dir: string): void => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (!entry.isDirectory()) continue
        emptyDirs(path.join(dir, entry.name))
        if (fs.readdirSync(path.join(dir, entry.name)).length === 0) fs.rmdirSync(path.join(dir, entry.name))
    }
}
emptyDirs(DIST)

fs.writeFileSync(path.join(TARGET, 'inventory.json'), JSON.stringify(inventoryOf(DIST, commit), null, 2) + '\n')
fs.writeFileSync(path.join(TARGET, 'files.ts'), filesModule(listFiles(DIST)))
console.log(`vendored ${wanted.size} files from zegadb/zega ${commit}`)
