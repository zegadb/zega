/**
 * Build `zega` as one executable per platform with `bun build --compile` (Bun
 * cross-compiles: every target is built from any host) into `dist/`, then print
 * each file's size. `bun run build [target...]` builds only the named targets.
 */
import fs from 'node:fs'
import path from 'node:path'

const ROOT = path.resolve(import.meta.dir, '..')

const TARGETS = [
    { target: 'bun-darwin-arm64', file: 'zega-darwin-arm64' },
    { target: 'bun-darwin-x64', file: 'zega-darwin-x64' },
    { target: 'bun-linux-x64', file: 'zega-linux-x64' },
    { target: 'bun-windows-x64', file: 'zega-windows-x64.exe' },
] as const

const wanted = process.argv.slice(2)
const chosen = wanted.length === 0 ? TARGETS : TARGETS.filter((t) => wanted.includes(t.target.replace('bun-', '')))
if (chosen.length === 0) {
    console.error(`no such target; the targets are ${TARGETS.map((t) => t.target.replace('bun-', '')).join(', ')}`)
    process.exit(1)
}

fs.mkdirSync(path.join(ROOT, 'dist'), { recursive: true })
let failed = false
for (const { target, file } of chosen) {
    const outfile = path.join(ROOT, 'dist', file)
    const build = Bun.spawnSync(
        ['bun', 'build', 'src/zega.ts', '--compile', `--target=${target}`, '--outfile', outfile],
        { cwd: ROOT, stdout: 'inherit', stderr: 'inherit' },
    )
    if (build.exitCode !== 0) {
        console.error(`${target}: build failed`)
        failed = true
        continue
    }
    const megabytes = (fs.statSync(outfile).size / 1e6).toFixed(1)
    console.log(`${file}  ${megabytes} MB`)
}
process.exit(failed ? 1 : 0)
