import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import pkg from '../package.json'
import { FAKE_SERVER, startCli } from './cli_helpers'
import { connectMcp, newHome } from './mcp_helpers'
import { FN, eventually, freePort, startDev, writeProject } from './dev_helpers'
import { FIXTURES, FUNCTION_ID, TOKEN, callBundle, deployedFunction, fakeApi, regionsBody, remove, scratch } from './helpers'
import { resolveServer } from '../src/handlers/server_binary'

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

    test('the compiled executable completes an MCP stdio handshake and answers tools/list', async () => {
        const isolated = newHome()
        const mcp = await connectMcp(['--groups', 'docs'], { home: isolated, api: 'http://127.0.0.1:9', command: [exe] })
        try {
            const tools = (await mcp.client.listTools()).tools.map((tool) => tool.name).sort()
            expect(tools).toEqual(['zega_docs_read', 'zega_docs_search'])
            expect(mcp.stderr()).not.toContain('token')
        } finally {
            await mcp.close()
            remove(isolated)
        }
    })

    test('talks to the API and prints its JSON; --human prints a table', async () => {
        const tokenFile = path.join(dir, 'token')
        fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
        const json = await run(['regions', '--api', api.url, '--token-file', tokenFile])
        expect(json.code).toBe(0)
        expect(JSON.parse(json.stdout).regions[0]).toMatchObject({ id: 'yyz', city: 'Toronto' })
        const result = await run(['regions', '--human', '--api', api.url, '--token-file', tokenFile])
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
        expect(result.stderr).toContain('missing --name')
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
    test.skipIf(process.platform === 'win32')('zega start runs the server it is pointed at, with the arguments and the exit code passed through', async () => {
        const fake = path.join(dir, 'fake-zega-server')
        fs.writeFileSync(fake, '#!/bin/sh\necho "got $*"\nexit 5\n', { mode: 0o755 })
        const result = await run(['start', '--human', '--data', './d', '--token-file', './t', '--server', fake])
        expect(result.stdout).toBe('got start --data ./d --token-file ./t\n')
        expect(result.code).toBe(5)
        // By default its lines are JSON events, so that everything on stdout parses.
        const json = await run(['start', '--data', './d', '--token-file', './t', '--server', fake])
        expect(json.stdout).toBe('{"event":"output","line":"got start --data ./d --token-file ./t"}\n')
        expect(json.code).toBe(5)
        expect(() => resolveServer({ roots: [path.join(dir, 'empty-lookup-root')] })).toThrow(
            /is not installed: it is the npm package @zegadb\/server-/,
        )
    })
    test.skipIf(process.platform === 'win32')('zega explorer serves the page embedded in the executable, on a database server it starts, and stops it again', async () => {
        const data = path.join(dir, 'explorer-data')
        const cli = startCli(['explorer', '--human', '--port', '0', '--server', FAKE_SERVER, '--data', data], { cwd: dir, home: dir, command: [exe] })
        await cli.until('Ctrl-C to stop', 'stderr')
        const url = cli.stdout().split('\n')[0]!
        expect(url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        const html = await (await fetch(url)).text()
        expect(html).toContain('<title>zega</title>')
        expect(html).not.toContain('zega.dev')
        const wasm = await fetch(`${url}/pkg/zega_wasm_bg.wasm`)
        expect(wasm.headers.get('content-type')).toBe('application/wasm')
        expect((await wasm.arrayBuffer()).byteLength).toBeGreaterThan(1_000_000)
        expect((await (await fetch(`${url}/graph`)).json())).toEqual({ ok: true, result: { nodes: [], rels: [] } })
        cli.signal('SIGTERM')
        expect(await cli.exited).toBe(0)
        expect(fs.existsSync(path.join(data, 'stopped'))).toBe(true)
    }, 30_000)

    test('zega build and zega dev work inside the executable: no bun on the PATH, a dependency, cloudflare:workers, a reload', async () => {
        const project = writeProject(
            {
                'src/index.ts': "import { env } from 'cloudflare:workers'\nimport { word } from './word'\nexport default { fetch: () => new Response(word + ' ' + env.NAME) }\n",
                'src/word.ts': "export const word = 'one'\n",
            },
            [{ id: FN, entry: 'src/index.ts' }],
        )
        fs.writeFileSync(path.join(project, '.dev.vars'), 'NAME=exe\n')
        const built = await run(['build'], { cwd: project, path: '/nonexistent' })
        expect(built.code).toBe(0)
        expect(fs.readFileSync(path.join(project, 'dist', `${FN}.js`), 'utf8')).toContain('cloudflare:workers')

        const port = freePort()
        const devRun = await startDev(['--port', String(port)], { cwd: project, home: dir, command: [exe], env: { PATH: '/nonexistent' } })
        try {
            await devRun.until('watching')
            const url = `http://127.0.0.1:${port}`
            expect(await (await fetch(url)).text()).toBe('one exe')
            fs.writeFileSync(path.join(project, 'src', 'word.ts'), "export const word = 'two'\n")
            expect(await eventually(url, (body) => body === 'two exe')).toBe('two exe')
        } finally {
            await devRun.stop()
            remove(project)
        }
    }, 30_000)
})
