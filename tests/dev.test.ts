import { afterAll, describe, expect, setDefaultTimeout, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FUNCTION_ID, TOKEN, fakeApi, graph, remove, scratch } from './helpers'
import { FN, SECOND, eventually, freePort, helloProject, startDev, writeProject, type Dev } from './dev_helpers'
import { MAX_VALUE_CHARS, parseDotenv } from '../src/handlers/dev_vars'
import { moduleWarnings, sourceWarnings } from '../src/handlers/workers_compat'

// A subprocess starts, bundles and serves; a slow machine needs more than the default five seconds.
setDefaultTimeout(30_000)

const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => remove(home))

const running: Dev[] = []
const dirs: string[] = []
afterAll(async () => {
    for (const dev of running) await dev.stop()
    for (const dir of dirs) remove(dir)
})

async function serve(dir: string, args: string[], env?: Record<string, string>) {
    dirs.push(dir)
    const port = freePort()
    const dev = await startDev([...args, '--port', String(port)], { cwd: dir, home, env })
    running.push(dev)
    await dev.until('watching')
    return { dev, port, url: `http://127.0.0.1:${port}` }
}

const echoEnv = `export default { fetch(request, env) { return Response.json({ keys: Object.keys(env).sort(), env, url: request.url }) } }\n`

describe('zega dev: serving', () => {
    test('serves a function that has a dependency and a local import, built as deploy builds it', async () => {
        const { dev, url } = await serve(helloProject(), [])
        const body = await (await fetch(`${url}/abc`)).text()
        expect(body).toMatch(/^HELLO FROM WORKERD \/ABC [A-Za-z0-9_-]{12}$/) // the `workerd` condition picked the Workers build
        expect(dev.stdout()).toContain(`${FN}  src/index.ts  ${url}`)
        expect(dev.stderr()).toMatch(/bundled src\/index\.ts: [\d,]+ bytes/)
        expect(dev.stderr()).toContain(`${FN} GET /abc 200`)
    })

    test('one stable port per function, in the order zega.json lists them, whether one or all are served', async () => {
        const dir = writeProject(
            { 'a.ts': "export default { fetch: () => new Response('a') }\n", 'b.ts': "export default { fetch: () => new Response('b') }\n" },
            [
                { id: FN, entry: 'a.ts' },
                { id: SECOND, entry: 'b.ts' },
            ],
        )
        const all = await serve(dir, [])
        expect(await (await fetch(all.url)).text()).toBe('a')
        expect(await (await fetch(`http://127.0.0.1:${all.port + 1}`)).text()).toBe('b')
        expect(all.dev.stdout()).toContain(`${SECOND}  b.ts  http://127.0.0.1:${all.port + 1}`)
        await all.dev.stop()

        // Only the second one: it keeps the port it has when all are served.
        const one = await serve(dir, [SECOND])
        expect(await (await fetch(`http://127.0.0.1:${one.port + 1}`)).text()).toBe('b')
    })

    test('a function that throws answers 500 and the server stays up', async () => {
        const dir = writeProject({ 'f.ts': "export default { fetch(r) { if (new URL(r.url).pathname === '/boom') throw new Error('kaboom'); return new Response('fine') } }\n" }, [
            { id: FN, entry: 'f.ts' },
        ])
        const { dev, url } = await serve(dir, [])
        const boom = await fetch(`${url}/boom`)
        expect(boom.status).toBe(500)
        expect(await (await fetch(`${url}/ok`)).text()).toBe('fine')
        expect(dev.stderr()).toContain(`${FN} threw: Error: kaboom`)
    })

    test('waitUntil work runs, and cloudflare:workers gives the function its own env', async () => {
        const dir = writeProject(
            {
                'f.ts': `import { env } from 'cloudflare:workers'
export default { fetch(request, e, ctx) {
  ctx.waitUntil(Promise.resolve().then(() => console.log('later: ' + env.NAME)))
  return new Response(env.NAME + '/' + e.NAME)
} }
`,
                'g.ts': `import { env } from 'cloudflare:workers'
export default { fetch() { return new Response(env.NAME) } }
`,
            },
            [
                { id: FN, entry: 'f.ts' },
                { id: SECOND, entry: 'g.ts' },
            ],
        )
        fs.writeFileSync(path.join(dir, '.dev.vars'), 'NAME=shared\n')
        fs.writeFileSync(path.join(dir, `.dev.vars.${SECOND}`), 'NAME=second\n')
        const { dev, port, url } = await serve(dir, [])
        expect(await (await fetch(url)).text()).toBe('shared/shared')
        expect(await (await fetch(`http://127.0.0.1:${port + 1}`)).text()).toBe('second') // two functions, two envs, one process
        await Bun.sleep(100)
        expect(dev.stdout()).toContain('later: shared')
    })
})

describe('zega dev: env and secrets', () => {
    test('env is .dev.vars, then .dev.vars.<id> over it; names are printed, values never; the shell and the token never reach it', async () => {
        const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
        fs.writeFileSync(path.join(dir, '.dev.vars'), '# a comment\nREGION=yyz\nexport API_KEY="s3cret value"\nPLAIN=one # trailing\n')
        fs.writeFileSync(path.join(dir, `.dev.vars.${FN}`), "API_KEY='from-the-function-file'\n")
        // A real token stored where `zega login` keeps it, and one in the shell.
        const config = path.join(home, '.config', 'zega')
        fs.mkdirSync(config, { recursive: true })
        fs.writeFileSync(path.join(config, 'cloud.json'), JSON.stringify({ api: 'https://cloud.zega.dev', token: TOKEN }))
        const { dev, url } = await serve(dir, [], { LEAKED_FROM_SHELL: 'shell-secret' })
        const body = (await (await fetch(url)).json()) as { keys: string[]; env: Record<string, string>; url: string }
        expect(body.env).toEqual({ REGION: 'yyz', API_KEY: 'from-the-function-file', PLAIN: 'one' })
        expect(JSON.stringify(body)).not.toContain('shell-secret')
        expect(JSON.stringify(body)).not.toContain(TOKEN)
        expect(dev.stdout()).toContain('env: API_KEY, PLAIN, REGION  (.dev.vars + .dev.vars.' + FN + ')')
        expect(dev.stdout() + dev.stderr()).not.toContain('from-the-function-file')
        expect(dev.stdout() + dev.stderr()).not.toContain('s3cret value')
        expect(dev.stdout() + dev.stderr()).not.toContain(TOKEN)
        fs.rmSync(path.join(config, 'cloud.json'))
    })

    test('a name or value Zega Cloud would refuse stops zega dev before it serves', async () => {
        const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
        dirs.push(dir)
        fs.writeFileSync(path.join(dir, '.dev.vars'), 'OK=1\nbad-name=2\n')
        const dev = await startDev([], { cwd: dir, home })
        expect(await dev.exited).toBe(1)
        expect(dev.stderr()).toContain('.dev.vars:2: "bad-name" is not a name Zega Cloud accepts')
        expect(dev.stdout()).toBe('')
    })

    test('parseDotenv: quotes, escapes, comments, export; and the cloud limits', () => {
        expect(parseDotenv('A=1\n\nB = two words  # c\nC="x\\ny # not a comment"\nD=\'raw\\n\'\nexport E=5\nF=\n', 'f')).toEqual({
            A: '1',
            B: 'two words',
            C: 'x\ny # not a comment',
            D: 'raw\\n',
            E: '5',
            F: '',
        })
        expect(() => parseDotenv('A="open', 'f')).toThrow('f:1: the quote is never closed')
        expect(() => parseDotenv('just text', 'f')).toThrow('f:1: expected NAME=value')
        expect(() => parseDotenv('A="x" y', 'f')).toThrow('text after the closing quote')
        expect(() => parseDotenv('1A=x', 'f')).toThrow('not a name Zega Cloud accepts')
        expect(MAX_VALUE_CHARS).toBe(5120)
    })
})

describe('zega dev: reload', () => {
    test('picks up a change to a file the entry imports, on the same port, and survives a broken save', async () => {
        const dir = writeProject({ 'src/index.ts': "import { word } from './word'\nexport default { fetch: () => new Response(word) }\n", 'src/word.ts': "export const word = 'one'\n" }, [
            { id: FN, entry: 'src/index.ts' },
        ])
        const { dev, url } = await serve(dir, [])
        expect(await (await fetch(url)).text()).toBe('one')

        fs.writeFileSync(path.join(dir, 'src', 'word.ts'), "export const word = 'two'\n")
        expect(await eventually(url, (body) => body === 'two')).toBe('two')
        expect(dev.stderr()).toContain(`reloaded ${FN} (src/index.ts)`)

        // A save that does not build: the old build keeps answering, and the error says why.
        fs.writeFileSync(path.join(dir, 'src', 'word.ts'), 'export const word = (\n')
        await dev.until('still serving the previous build', 'stderr')
        expect(await (await fetch(url)).text()).toBe('two')

        fs.writeFileSync(path.join(dir, 'src', 'word.ts'), "export const word = 'three'\n")
        expect(await eventually(url, (body) => body === 'three')).toBe('three')
    })

    test('a change to .dev.vars reloads the env', async () => {
        const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
        fs.writeFileSync(path.join(dir, '.dev.vars'), 'A=1\n')
        const { url } = await serve(dir, [])
        expect(((await (await fetch(url)).json()) as { env: object }).env).toEqual({ A: '1' })
        fs.writeFileSync(path.join(dir, '.dev.vars'), 'A=2\nB=3\n')
        await eventually(url, (body) => (JSON.parse(body) as { env: Record<string, string> }).env.B === '3')
    })

    test('--no-watch serves once and does not rebuild', async () => {
        const dir = writeProject({ 'f.ts': "export default { fetch: () => new Response('one') }\n" }, [{ id: FN, entry: 'f.ts' }])
        const { dev, url } = await serve(dir, ['--no-watch'])
        expect(dev.stdout()).toContain('not watching for changes')
        fs.writeFileSync(path.join(dir, 'f.ts'), "export default { fetch: () => new Response('two') }\n")
        await Bun.sleep(400)
        expect(await (await fetch(url)).text()).toBe('one')
    })
})

describe('zega dev: which graph', () => {
    test('--local-graph points the graph variable at a local zega-server and says it is the LOCAL one', async () => {
        const local = fakeApi({ 'GET /health': { body: { ok: true } } })
        try {
            const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
            const { dev, url } = await serve(dir, [`--local-graph=${local.url}`])
            const body = (await (await fetch(url)).json()) as { env: Record<string, string> }
            expect(body.env).toEqual({ ZEGA_GRAPH_URL: local.url })
            expect(dev.stdout()).toContain(`graph: LOCAL zega-server at ${local.url} as ZEGA_GRAPH_URL`)
            expect(dev.stdout()).not.toContain('nothing answers')
        } finally {
            local.stop()
        }
    })

    test('--local-graph with nothing listening says so; a name the user chose is used; a non-local address is refused', async () => {
        const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
        const dead = `http://127.0.0.1:${freePort()}`
        const { dev, url } = await serve(dir, [`--local-graph=${dead}`, '--graph-url-var', 'DB_URL'])
        expect(((await (await fetch(url)).json()) as { env: object }).env).toEqual({ DB_URL: dead })
        expect(dev.stdout()).toContain('(nothing answers there yet: start it with `zega start`)')

        const remote = await startDev(['--local-graph=https://x.zegadb.com'], { cwd: dir, home })
        expect(await remote.exited).toBe(1)
        expect(remote.stderr()).toContain('use --cloud-graph <id>')
    })

    test('--cloud-graph resolves the address with the stored token, says it is CLOUD, and never gives the token or the graph to anything else', async () => {
        const api = fakeApi({ [`GET /v1/graphs/${graph.id}`]: { body: { graph } } })
        try {
            const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
            fs.writeFileSync(path.join(dir, '.dev.vars'), 'ZEGA_GRAPH_KEY=zk_local_test\nZEGA_GRAPH_URL=http://stale.example\n')
            const { dev, url } = await serve(dir, ['--cloud-graph', graph.id, '--api', api.url, '--token-file', tokenFile])
            const body = (await (await fetch(url)).json()) as { env: Record<string, string> }
            expect(body.env).toEqual({ ZEGA_GRAPH_KEY: 'zk_local_test', ZEGA_GRAPH_URL: graph.hostUrl })
            expect(JSON.stringify(body)).not.toContain(TOKEN)
            expect(api.seen).toHaveLength(1)
            expect(api.seen[0]).toMatchObject({ method: 'GET', pathname: `/v1/graphs/${graph.id}`, authorization: `Bearer ${TOKEN}` })
            expect(dev.stdout()).toContain(`graph: CLOUD graph ${graph.id} "${graph.name}" at ${graph.hostUrl} as ZEGA_GRAPH_URL`)
            expect(dev.stderr()).toContain('ZEGA_GRAPH_URL from .dev.vars is replaced by the graph you chose')
            expect(dev.stderr()).not.toContain('no ZEGA_GRAPH_KEY')
        } finally {
            api.stop()
        }
    })

    test('--cloud-graph without a key in .dev.vars warns; with --local-graph too it is refused; by default the start is one `ready` event', async () => {
        const api = fakeApi({ [`GET /v1/graphs/${graph.id}`]: { body: { graph } } })
        try {
            const dir = writeProject({ 'f.ts': echoEnv }, [{ id: FN, entry: 'f.ts' }])
            dirs.push(dir)
            const both = await startDev(['--cloud-graph', graph.id, '--local-graph', '--api', api.url, '--token-file', tokenFile], { cwd: dir, home })
            expect(await both.exited).toBe(1)
            expect(both.stderr()).toContain('--local-graph and --cloud-graph are alternatives')

            const port = freePort()
            const dev = await startDev(['--cloud-graph', graph.id, '--api', api.url, '--token-file', tokenFile, '--port', String(port)], { cwd: dir, home, human: false })
            running.push(dev)
            await dev.until('"watching":true')
            expect(JSON.parse(dev.stdout().split('\n')[0]!)).toEqual({
                event: 'ready',
                functions: [{ id: FN, entry: 'f.ts', url: `http://127.0.0.1:${port}`, env: ['ZEGA_GRAPH_URL'] }],
                graph: { kind: 'cloud', url: graph.hostUrl, id: graph.id, name: graph.name, urlVar: 'ZEGA_GRAPH_URL' },
                watching: true,
            })
            await dev.until('no ZEGA_GRAPH_KEY in .dev.vars')
            const events = dev.stdout().trim().split('\n').map((line) => JSON.parse(line))
            expect(events[1]).toEqual({ event: 'warning', function: FN, message: `${FN}: no ZEGA_GRAPH_KEY in .dev.vars; the cloud graph will answer 401` })
            expect(dev.stderr()).toBe('')
        } finally {
            api.stop()
        }
    })
})

describe('zega dev: what Bun is not', () => {
    test('warns, in one line each, about Workers-only APIs the function uses and handlers it defines', async () => {
        const dir = writeProject(
            {
                'f.ts': `export default {
  fetch(request) { return new Response(String(request.cf?.colo) + typeof caches) },
  scheduled() {},
}
export class Counter {}
`,
            },
            [{ id: FN, entry: 'f.ts' }],
        )
        const { dev, url } = await serve(dir, [])
        expect(await (await fetch(url)).text()).toBe('undefinedundefined')
        const lines = dev.stderr().split('\n').filter((line) => line.startsWith('warning: f.ts:'))
        expect(lines.some((l) => l.includes('`request.cf` is not set by zega dev'))).toBe(true)
        expect(lines.some((l) => l.includes('defines scheduled'))).toBe(true)
        expect(lines.some((l) => l.includes('exports Counter besides the default'))).toBe(true)
    })

    test('a function that is clean is not warned about, and Node built-ins are refused as deploy refuses them', async () => {
        const clean = await serve(helloProject(), [])
        expect(clean.dev.stderr()).not.toContain('warning')

        const dir = writeProject({ 'f.ts': "import { createHash } from 'node:crypto'\nexport default { fetch: () => new Response(typeof createHash) }\n" }, [{ id: FUNCTION_ID, entry: 'f.ts' }])
        dirs.push(dir)
        const dev = await startDev([], { cwd: dir, home })
        expect(await dev.exited).toBe(1)
        expect(dev.stderr()).toContain('Node built-ins are not available in Zega Cloud functions yet: node:crypto')
    })

    test('the checks themselves', () => {
        expect(sourceWarnings('const x = 1')).toEqual([])
        expect(sourceWarnings('new WebSocketPair(); caches.default.match(r); ctx.passThroughOnException()')).toHaveLength(3)
        expect(sourceWarnings('process.env.TOKEN')).toHaveLength(1)
        expect(moduleWarnings({ default: { fetch() {}, queue() {}, email() {} } })).toEqual([
            'defines queue, email: a function is only invoked over HTTP, so zega dev never calls them',
        ])
    })

    test('a function with no fetch, and a port that is taken, are named', async () => {
        const dir = writeProject({ 'f.ts': 'export default { hello() {} }\n' }, [{ id: FN, entry: 'f.ts' }])
        dirs.push(dir)
        const none = await startDev([], { cwd: dir, home })
        expect(await none.exited).toBe(1)
        expect(none.stderr()).toContain('f.ts: the default export has no fetch(request, env, ctx) function')

        const taken = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('') })
        try {
            const okDir = writeProject({ 'f.ts': 'export default { fetch: () => new Response("x") }\n' }, [{ id: FN, entry: 'f.ts' }])
            dirs.push(okDir)
            const busy = await startDev(['--port', String(taken.port)], { cwd: okDir, home })
            expect(await busy.exited).toBe(1)
            expect(busy.stderr()).toContain(`port ${taken.port} is already in use (for ${FN}); pick another with --port`)
        } finally {
            taken.stop(true)
        }
    })
})

describe('zega dev: by default every line is a JSON event on stdout', () => {
    /** Start without --human and read the events printed so far. */
    async function json(dir: string, args: string[] = []) {
        dirs.push(dir)
        const port = freePort()
        const dev = await startDev([...args, '--port', String(port)], { cwd: dir, home, human: false })
        running.push(dev)
        await dev.until('"event":"ready"')
        const events = () =>
            dev
                .stdout()
                .trim()
                .split('\n')
                .map((line) => JSON.parse(line) as Record<string, unknown>)
        return { dev, url: `http://127.0.0.1:${port}`, port, events }
    }

    test('ready, then a request event per request, then threw for a function that throws; nothing else is printed', async () => {
        const dir = writeProject({ 'f.ts': "export default { fetch(r) { if (new URL(r.url).pathname === '/boom') throw new Error('kaboom'); return new Response('fine') } }\n" }, [
            { id: FN, entry: 'f.ts' },
        ])
        const { dev, url, port, events } = await json(dir)
        expect(events()[0]).toEqual({
            event: 'ready',
            functions: [{ id: FN, entry: 'f.ts', url, env: [] }],
            graph: null,
            watching: true,
        })
        expect(await (await fetch(`${url}/ok`)).text()).toBe('fine')
        expect((await fetch(`${url}/boom`)).status).toBe(500)
        await dev.until('"path":"/boom"')
        const seen = events()
        expect(seen.find((e) => e.event === 'request' && e.path === '/ok')).toMatchObject({ function: FN, method: 'GET', status: 200 })
        expect(seen.find((e) => e.event === 'request' && e.path === '/boom')).toMatchObject({ status: 500 })
        expect(seen.find((e) => e.event === 'threw')).toMatchObject({ function: FN })
        expect(String(seen.find((e) => e.event === 'threw')?.message)).toContain('kaboom')
        // No banner, no logo, no "bundled" line: stdout is events and stderr is empty.
        expect(dev.stdout()).not.toContain('▐')
        expect(dev.stdout()).not.toContain('watching for changes')
        expect(dev.stderr()).toBe('')
        expect(port).toBeGreaterThan(0)
    })

    test('a change is a reloaded event; a broken save is a build_failed event and the old build keeps serving', async () => {
        const dir = writeProject({ 'f.ts': "export default { fetch: () => new Response('one') }\n" }, [{ id: FN, entry: 'f.ts' }])
        const { dev, url, events } = await json(dir)
        fs.writeFileSync(path.join(dir, 'f.ts'), "export default { fetch: () => new Response('two') }\n")
        await dev.until(`"event":"reloaded","function":"${FN}"`)
        expect(events().find((e) => e.event === 'reloaded')).toEqual({ event: 'reloaded', function: FN, entry: 'f.ts' })
        fs.writeFileSync(path.join(dir, 'f.ts'), 'export default (\n')
        await dev.until('"event":"build_failed"')
        expect(await (await fetch(url)).text()).toBe('two')
        expect(events().find((e) => e.event === 'build_failed')).toMatchObject({ function: FN })
        expect(dev.stderr()).toBe('')
    })
})
