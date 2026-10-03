import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import pkg from '../package.json'
import { FIXTURES, FUNCTION_ID, TOKEN, callBundle, deployedFunction, fakeApi, regionsBody, remove, scratch } from './helpers'

/**
 * The compiled single-file executable, not the TypeScript: what ships is what
 * `bun build --compile` makes, so that is what is run (host platform only; the
 * four targets are built by `bun run build`).
 */
const ROOT = path.resolve(import.meta.dir, '..')
const dir = scratch()
const exe = path.join(dir, process.platform === 'win32' ? 'zega.exe' : 'zega')
const api = fakeApi({ 'GET /v1/regions': { body: regionsBody } })

beforeAll(() => {
    const build = Bun.spawnSync(['bun', 'build', 'src/zega.ts', '--compile', '--outfile', exe], {
        cwd: ROOT,
        stdout: 'pipe',
        stderr: 'pipe',
    })
    if (build.exitCode !== 0) throw new Error(`compile failed: ${build.stderr.toString()}`)
}, 120_000)

afterAll(() => {
    api.stop()
    remove(dir)
})

async function run(args: string[], options: { cwd?: string; path?: string } = {}) {
    const child = Bun.spawn([exe, ...args], {
        cwd: options.cwd ?? dir,
        env: { PATH: options.path ?? process.env.PATH ?? '', HOME: dir, XDG_CONFIG_HOME: path.join(dir, '.config') },
        stdin: 'ignore',
        stdout: 'pipe',
        stderr: 'pipe',
    })
    const [stdout, stderr, code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
    ])
    return { stdout, stderr, code }
}

describe('the compiled executable', () => {
    test('reports the package version', async () => {
        const result = await run(['--version'])
        expect(result.code).toBe(0)
        expect(result.stdout.trim()).toBe(pkg.version)
    })

    test('talks to the API and prints a table', async () => {
        const tokenFile = path.join(dir, 'token')
        fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
        const result = await run(['regions', '--api', api.url, '--token-file', tokenFile])
        expect(result.code).toBe(0)
        expect(result.stdout).toContain('yyz  Toronto')
        expect(api.seen.at(-1)?.authorization).toBe(`Bearer ${TOKEN}`)
    })

    test('init works without a terminal, from flags', async () => {
        const result = await run(['init', '--name', 'compiled', '--region', 'yyz'])
        expect(result.code).toBe(0)
        expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).name).toBe('compiled')
    })

    test('a missing flag fails naming it', async () => {
        const result = await run(['init', '--force'])
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('missing --name, --region')
    })

    test('bundles a function with an npm dependency INSIDE the executable (no bun on the PATH) and uploads one ES module', async () => {
        const tokenFile = path.join(dir, 'token')
        fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
        const functionApi = fakeApi({ [`PUT /v1/functions/${FUNCTION_ID}/code`]: { body: { function: deployedFunction } } })
        try {
            const result = await run(
                ['function', 'deploy', FUNCTION_ID, 'src/index.ts', '--api', functionApi.url, '--token-file', tokenFile],
                // A PATH with no bun in it: the bundler is the executable's own.
                { cwd: path.join(FIXTURES, 'hello'), path: '/nonexistent' },
            )
            expect(result.stderr).not.toContain('could not')
            expect(result.code).toBe(0)
            expect(functionApi.seen).toHaveLength(1)
            const request = functionApi.seen[0]!
            expect(request).toMatchObject({ method: 'PUT', authorization: `Bearer ${TOKEN}`, contentType: 'application/javascript' })
            // One ES module that exports a default fetch handler and contains the dependency's code.
            expect(request.body).toMatch(/\bas default\b|\bexport default\b/)
            expect(request.body).toContain('useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict')
            expect(request.body).toContain('hello from workerd')
            expect(request.body).not.toMatch(/^import /m)
            expect(await callBundle(request.body, 'https://f.example/abc')).toMatch(/^HELLO FROM WORKERD \/ABC [A-Za-z0-9_-]{12}$/)
        } finally {
            functionApi.stop()
        }
    })

    test('refuses a Node built-in inside the executable too', async () => {
        const result = await run(['function', 'deploy', FUNCTION_ID, 'src/crypto.ts'], {
            cwd: path.join(FIXTURES, 'builtins'),
            path: '/nonexistent',
        })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('Node built-ins are not available in Zega Cloud functions yet: node:crypto in src/crypto.ts')
    })
})
