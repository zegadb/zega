import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { read as readConfig } from '../src/handlers/project_config'
import { TEMPLATES, packageName, starter } from '../src/templates'
import { ZEGA_LIB } from '../src/templates/versions'
import { eventually, startDev } from './dev_helpers'
import { FIXTURES, TOKEN, deployedFunction, errorOf, fakeApi, remove, run, scratch } from './helpers'

const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => remove(home))

const STAND_IN = path.join(FIXTURES, 'zega-lib-stand-in')
const FN = 'f0bbbbbbbbbbbbbbbb'

/** `zega init` with flags (no terminal), in a fresh directory; the directory and what it printed. */
async function init(template: string | null, extra: string[] = []) {
    const dir = scratch()
    const args = ['init', '--here', '--name', 'My Starter', '--region', 'yyz', ...(template === null ? [] : ['--template', template]), ...extra]
    const result = await run(args, { home, cwd: dir })
    return { dir, result }
}

const readJson = (file: string) => JSON.parse(fs.readFileSync(file, 'utf8'))

/** What each starter must write (besides zega.json, which every one has). */
const FILES: Record<string, string[]> = {
    function: ['package.json', 'src/index.ts', 'tsconfig.json', '.env.example', '.gitignore', 'README.md'],
    nextjs: ['package.json', 'next.config.ts', 'open-next.config.ts', 'tsconfig.json', 'app/layout.tsx', 'app/page.tsx', '.env.example', '.gitignore', 'README.md'],
    astro: ['package.json', 'astro.config.mjs', 'tsconfig.json', 'src/pages/index.astro', '.env.example', '.gitignore', 'README.md'],
}

describe('the starters', () => {
    for (const template of TEMPLATES) {
        test(`${template}: writes its files, lists them in the JSON result, and depends on @zegadb/lib at the pinned version`, async () => {
            const { dir, result } = await init(template)
            try {
                expect(result.code).toBe(0)
                const printed = JSON.parse(result.stdout)
                expect(printed.template).toBe(template)
                expect([...printed.files].sort()).toEqual(['zega.json', ...FILES[template]!].sort())
                for (const file of printed.files) expect(fs.existsSync(path.join(dir, file))).toBe(true)
                expect(printed.next.length).toBeGreaterThan(2)
                expect(printed.next[0]).toBe('bun install')
                if (template === 'nextjs' || template === 'astro') {
                    expect(printed.next).toContain('bun run deploy')
                    expect(printed.next.join('\n')).not.toContain('not supported')
                }
                // The client library, as the one dependency the code imports (and exactly pinned).
                const pkg = readJson(path.join(dir, 'package.json'))
                expect(pkg.dependencies['@zegadb/lib']).toBe(ZEGA_LIB)
                expect(pkg.name).toBe('my-starter')
                expect(pkg.scripts.dev).toBe('zega dev')
                // The env example has names and no values; the real file is never committed.
                expect(fs.readFileSync(path.join(dir, '.env.example'), 'utf8')).toMatch(/^ZEGA_GRAPH_URL=$/m)
                expect(fs.readFileSync(path.join(dir, '.env.example'), 'utf8')).toMatch(/^ZEGA_GRAPH_KEY=$/m)
                expect(fs.readFileSync(path.join(dir, '.gitignore'), 'utf8').split('\n')).toContain('.env')
                // zega.json is what the CLI itself reads.
                expect(readConfig(dir)).toMatchObject({ version: 1, name: 'My Starter', region: 'yyz' })
            } finally {
                remove(dir)
            }
        })
    }

    test('the function is the default and is ready to build before it has a Cloud id', async () => {
        const plain = await init(null)
        try {
            expect(JSON.parse(plain.result.stdout).template).toBe('function')
            expect(readJson(path.join(plain.dir, 'zega.json')).functions).toEqual([{ name: 'My Starter', entry: 'src/index.ts' }])
            expect(JSON.parse(plain.result.stdout).next.join('\n')).toContain('bun run dev')
        } finally {
            remove(plain.dir)
        }
        const listed = await init('function', ['--function', FN])
        try {
            expect(readJson(path.join(listed.dir, 'zega.json')).functions).toEqual([{ id: FN, entry: 'src/index.ts' }])
            expect(fs.readFileSync(path.join(listed.dir, 'src', 'index.ts'), 'utf8')).toContain("import { connect } from '@zegadb/lib/client'")
            const pkg = readJson(path.join(listed.dir, 'package.json'))
            expect(pkg.scripts).toEqual({ dev: 'zega dev', build: 'zega build', deploy: 'zega deploy' })
        } finally {
            remove(listed.dir)
        }
    })

    test('init defaults the region to yyz and says which region it chose', async () => {
        const dir = scratch()
        try {
            const result = await run(['init', '--here', '--human', '--name', 'region-default'], { home, cwd: dir })
            expect(result.code).toBe(0)
            expect(readJson(path.join(dir, 'zega.json')).region).toBe('yyz')
            expect(result.stdout).toContain('Region    yyz')
        } finally { remove(dir) }
    })

    test('the app starters carry what a build and a deploy need in zega.json: kind, framework, build paths and the runtime flags', async () => {
        for (const [template, worker, assets] of [
            ['nextjs', '.open-next/worker.js', '.open-next/assets'],
            ['astro', 'dist/server/entry.mjs', 'dist/client'],
        ] as const) {
            const { dir } = await init(template)
            try {
                const config = readJson(path.join(dir, 'zega.json'))
                expect(config).toMatchObject({ kind: 'app', framework: template, build: { worker, assets } })
                expect(config.build.command.length).toBeGreaterThan(1)
                expect(config.runtime).toEqual({ compatibilityDate: '2026-09-01', compatibilityFlags: ['nodejs_compat'] })
                expect(config.functions).toBeUndefined()
                // `bun run build` is the same command, so the script and zega.json cannot differ.
                expect(readJson(path.join(dir, 'package.json')).scripts.build).toBe(config.build.command.join(' '))
                // No wrangler file to keep, and no auth package, placeholder or stub.
                expect(fs.readdirSync(dir).some((name) => name.startsWith('wrangler'))).toBe(false)
                const all = Object.values(starter(template, { name: 'x', entry: 'src/index.ts' }).files).join('\n')
                expect(all).not.toMatch(/@zegadb\/auth|zega-auth/)
                expect(fs.readFileSync(path.join(dir, 'README.md'), 'utf8').split('\n').filter((line) => /^Auth:/.test(line))).toHaveLength(1)
            } finally {
                remove(dir)
            }
        }
    })

    test('--template rejects a name that is not one, names the templates, and writes nothing', async () => {
        const { dir, result } = await init('react')
        try {
            expect(result.code).toBe(1)
            expect(result.stdout).toBe('')
            expect(errorOf(result)).toMatchObject({ code: 'invalid_flag' })
            expect(errorOf(result).message).toContain('--template: "react" is not a template; the templates are function, nextjs, astro')
            expect(fs.readdirSync(dir)).toEqual([])
        } finally {
            remove(dir)
        }
    })

    test('--function and --entry are for the function starter only', async () => {
        const { dir, result } = await init('astro', ['--function', FN])
        try {
            expect(result.code).toBe(1)
            expect(errorOf(result).message).toContain('--function and --entry are for the function starter, not astro')
            expect(fs.readdirSync(dir)).toEqual([])
        } finally {
            remove(dir)
        }
    })

    test('nothing in the way is overwritten without --force: the files are named, and --force replaces them', async () => {
        const { dir } = await init('nextjs')
        try {
            fs.writeFileSync(path.join(dir, 'app', 'page.tsx'), 'mine\n')
            const again = await run(['init', '--here', '--name', 'x', '--region', 'yyz', '--template', 'nextjs'], { home, cwd: dir })
            expect(again.code).toBe(1)
            expect(errorOf(again).message).toContain('zega.json, package.json')
            expect(errorOf(again).message).toContain('app/page.tsx')
            expect(fs.readFileSync(path.join(dir, 'app', 'page.tsx'), 'utf8')).toBe('mine\n')
            const forced = await run(['init', '--here', '--name', 'x', '--region', 'yyz', '--template', 'nextjs', '--force'], { home, cwd: dir })
            expect(forced.code).toBe(0)
            expect(fs.readFileSync(path.join(dir, 'app', 'page.tsx'), 'utf8')).toContain('@zegadb/lib/client')
        } finally {
            remove(dir)
        }
    })

    test('the package name is made from the project name', () => {
        expect(packageName('My Starter!')).toBe('my-starter')
        expect(packageName('@scope/App')).toBe('scope-app')
        expect(packageName('   ')).toBe('zega-app')
    })
})

describe('the function starter builds', () => {
    test('a fresh local function builds offline and deploy creates it, records its id, then uploads', async () => {
        const { dir, result } = await init('function')
        const id = 'f0abcdefghjklmnopq'
        const projectId = 'p0project123'
        const api = fakeApi({
            [`POST /v1/projects/${projectId}/functions`]: { status: 201, body: { function: { id } } },
            [`PUT /v1/functions/${id}/code`]: { body: { function: { ...deployedFunction, id, name: 'My Starter' } } },
        })
        try {
            expect(result.code).toBe(0)
            fs.cpSync(STAND_IN, path.join(dir, 'node_modules', '@zegadb', 'lib'), { recursive: true })
            const built = await run(['build'], { home, cwd: dir })
            expect(built.code).toBe(0)
            expect(JSON.parse(built.stdout)[0]).toMatchObject({ id: 'My Starter', entry: 'src/index.ts' })
            const missingProject = await run(['deploy'], { home, cwd: dir })
            expect(missingProject.code).toBe(1)
            expect(JSON.parse(missingProject.stderr).error).toMatchObject({ code: 'missing_flag' })
            expect(missingProject.stderr).toContain('--project <project id>')
            const deployed = await run(['deploy', '--project', projectId, '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(deployed.code).toBe(0)
            expect(api.seen.map((request) => `${request.method} ${request.pathname}`)).toEqual([
                `POST /v1/projects/${projectId}/functions`, `PUT /v1/functions/${id}/code`,
            ])
            expect(JSON.parse(api.seen[0]!.body)).toEqual({ name: 'My Starter' })
            expect(readJson(path.join(dir, 'zega.json'))).toMatchObject({ projectId, functions: [{ id, entry: 'src/index.ts' }] })
        } finally { api.stop(); remove(dir) }
    })

    test('zega build bundles it with the client import, and the bundle runs against a graph', async () => {
        const { dir, result } = await init('function', ['--function', FN])
        const graph = Bun.serve({
            hostname: '127.0.0.1',
            port: 0,
            async fetch(request) {
                const body = (await request.json()) as { query: string }
                return Response.json({ ok: true, result: { Person: [{ name: 'Ada' }], echoed: body.query, auth: request.headers.get('authorization') } })
            },
        })
        try {
            expect(result.code).toBe(0)
            // @zegadb/lib with /client is not on npm yet: a stand-in with the same entry stands where it will be.
            fs.cpSync(STAND_IN, path.join(dir, 'node_modules', '@zegadb', 'lib'), { recursive: true })
            const built = await run(['build'], { home, cwd: dir })
            expect(built.code).toBe(0)
            expect(JSON.parse(built.stdout)[0]).toMatchObject({ id: FN, entry: 'src/index.ts', file: `dist/${FN}.js` })
            const code = fs.readFileSync(path.join(dir, 'dist', `${FN}.js`), 'utf8')
            expect(code).not.toMatch(/^import /m) // the client is inside the one module
            const module = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
            const answer = await module.default.fetch(new Request('https://f.example/'), {
                ZEGA_GRAPH_URL: `http://127.0.0.1:${graph.port}`,
                ZEGA_GRAPH_KEY: 'zk_test',
            })
            expect(await answer.json()).toMatchObject({ Person: [{ name: 'Ada' }], echoed: '{ Person { name } }', auth: 'Bearer zk_test' })
        } finally {
            graph.stop(true)
            remove(dir)
        }
    })
})

describe('app config', () => {
    test('reads and retains the project and app ids written by app deploy', () => {
        const dir = scratch()
        try {
            fs.writeFileSync(
                path.join(dir, 'zega.json'),
                JSON.stringify({
                    version: 1,
                    name: 'x',
                    region: 'yyz',
                    kind: 'app',
                    framework: 'astro',
                    build: { command: ['astro', 'build'], worker: 'dist/server/entry.mjs', assets: 'dist/client' },
                    runtime: { compatibilityDate: '2026-09-01', compatibilityFlags: ['nodejs_compat'] },
                    projectId: 'p0project',
                    appId: FN,
                }),
            )
            expect(readConfig(dir)).toMatchObject({ projectId: 'p0project', appId: FN })
            fs.writeFileSync(path.join(dir, 'zega.json'), JSON.stringify({ version: 1, name: 'x', region: 'yyz', appId: FN }))
            expect(() => readConfig(dir)).toThrow('`appId` is only valid when `kind` is "app"')
        } finally {
            remove(dir)
        }
    })

    test('a zega.json that says app but not how it is built is refused where it is read', () => {
        const dir = scratch()
        try {
            fs.writeFileSync(path.join(dir, 'zega.json'), JSON.stringify({ version: 1, name: 'x', region: 'yyz', kind: 'app', framework: 'nextjs' }))
            expect(() => readConfig(dir)).toThrow('`build` must have a `command` (a list of words), a `worker` and an `assets` path')
            fs.writeFileSync(path.join(dir, 'zega.json'), JSON.stringify({ version: 1, name: 'x', region: 'yyz', kind: 'app', framework: 'rails' }))
            expect(() => readConfig(dir)).toThrow('`framework` must be one of nextjs, astro')
        } finally {
            remove(dir)
        }
    })
})

/**
 * The app starters, installed and built for real. They need the network (`bun install` fetches Next.js, Astro,
 * OpenNext and the Cloudflare adapter), so set SKIP_NETWORK_TESTS=1 to leave them out.
 */
describe.skipIf(process.env.SKIP_NETWORK_TESTS === '1')('app starters: installed, built and run (network)', () => {
    /** Scaffold, install the packages that are on npm, and put the stand-in where @zegadb/lib will be. */
    async function installed(template: 'nextjs' | 'astro') {
        const { dir } = await init(template)
        const pkgFile = path.join(dir, 'package.json')
        const pkg = readJson(pkgFile)
        // Neither @zegadb/lib@<version> (with /client) nor this CLI is on npm yet.
        delete pkg.dependencies['@zegadb/lib']
        delete pkg.devDependencies.zegadb
        fs.writeFileSync(pkgFile, JSON.stringify(pkg, null, 2))
        const install = Bun.spawnSync(['bun', 'install'], { cwd: dir, stdout: 'pipe', stderr: 'pipe', env: { PATH: process.env.PATH ?? '', HOME: home } })
        if (install.exitCode !== 0) throw new Error(`bun install failed: ${install.stderr.toString()}`)
        fs.cpSync(STAND_IN, path.join(dir, 'node_modules', '@zegadb', 'lib'), { recursive: true })
        return dir
    }

    for (const [template, worker, assets] of [
        ['nextjs', '.open-next/worker.js', '.open-next/assets'],
        ['astro', 'dist/server/entry.mjs', 'dist/client'],
    ] as const) {
        test(`${template}: zega build runs the framework build and reports the worker and the assets; zega dev serves a page that reads the graph`, async () => {
            const dir = await installed(template)
            const graph = Bun.serve({
                hostname: '127.0.0.1',
                port: 0,
                fetch: async (request) => Response.json({ ok: true, result: { Person: [{ name: 'Ada Lovelace' }], auth: request.headers.get('authorization') } }),
            })
            try {
                const built = await run(['build'], { home, cwd: dir })
                expect(built.stderr).not.toContain('"error"')
                expect(built.code).toBe(0)
                const printed = JSON.parse(built.stdout)
                expect(printed).toMatchObject({ kind: 'app', framework: template, worker, assets })
                expect(printed.workerBytes).toBeGreaterThan(0)
                expect(fs.statSync(path.join(dir, worker)).isFile()).toBe(true)
                expect(fs.statSync(path.join(dir, assets)).isDirectory()).toBe(true)
                // --human says it in a line.
                const again = await run(['build', '--human'], { home, cwd: dir })
                expect(again.stdout).toContain(`Built  ${template} app: worker ${worker}`)

                // zega dev: the framework's own dev server, with .dev.vars in its environment.
                fs.writeFileSync(path.join(dir, '.env'), `ZEGA_GRAPH_URL=http://127.0.0.1:${graph.port}\nZEGA_GRAPH_KEY=zk_from_dev_vars\n`)
                const port = 15000 + Math.floor(Math.random() * 3000)
                const dev = await startDev(['--port', String(port)], { cwd: dir, home, human: false })
                try {
                    await dev.until('"event":"starting"')
                    const page = await eventually(`http://localhost:${port}/`, (body) => body.includes('Ada Lovelace'))
                    expect(page).toContain('Ada Lovelace')
                    expect(dev.stdout().split('\n')[0]).toContain('"env":["ZEGA_GRAPH_KEY","ZEGA_GRAPH_URL"]')
                    expect(dev.stdout()).not.toContain('zk_from_dev_vars')
                } finally {
                    await dev.stop()
                }
            } finally {
                graph.stop(true)
                remove(dir)
            }
        }, 300_000)
    }
})
