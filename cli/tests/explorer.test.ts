import { afterAll, describe, expect, setDefaultTimeout, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FAKE_SERVER, FIXTURE_ASSETS, recorded, startCli, workdir, type Cli } from './cli_helpers'
import { TOKEN, fakeApi, graph, remove } from './helpers'
import { dashboardFor } from '../src/core/explorer/explorer'
import { explorer as runExplorer } from '../src/core/explorer/explorer'
import { directoryAssets } from '../src/handlers/explorer_assets'
import { serveExplorer, type ExplorerOptions } from '../src/handlers/explorer_server'
import { resolveServer, serverPackage } from '../src/handlers/server_binary'

setDefaultTimeout(30_000)
const posix = process.platform !== 'win32'

const work = workdir()
const home = path.join(work, 'home')
const tokenFile = path.join(work, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
const running: Cli[] = []
afterAll(async () => {
    for (const cli of running) {
        cli.signal('SIGTERM')
        await cli.exited
    }
    remove(work)
})

let counter = 0
/** A fresh data directory name under the test's own. */
const dataDir = (name = `data${++counter}`) => path.join(work, name)

type Started = { cli: Cli; url: string; data: string }
/**
 * Start `zega explorer` on the fake database server and the fixture page, on a port of the OS's choosing.
 * It reads as a person does (`--human`: the address on a line, the rest on stderr) unless `human: false`.
 */
async function explorer(extra: string[] = [], options: { data?: string; assets?: boolean; human?: boolean } = {}): Promise<Started> {
    const data = options.data ?? dataDir()
    const args = ['explorer', ...(options.human === false ? [] : ['--human']), '--port', '0', '--server', FAKE_SERVER, '--data', data, ...(options.assets === false ? [] : ['--assets', FIXTURE_ASSETS]), ...extra]
    const cli = startCli(args, { cwd: work, home })
    running.push(cli)
    await cli.until('\n')
    return { cli, data, url: cli.stdout().split('\n')[0]! }
}

describe.skipIf(!posix)('zega explorer: a local database', () => {
    test('prints its address first (and opens nothing), serves the page, and says what it serves on stderr', async () => {
        const { cli, url, data } = await explorer()
        expect(url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        expect(cli.stdout()).toBe(`${url}\n`)
        await cli.until('Ctrl-C to stop', 'stderr')
        expect(cli.stderr()).toContain(`explorer for ${data}`)
        const page = await fetch(url)
        expect(page.status).toBe(200)
        expect(page.headers.get('content-type')).toBe('text/html; charset=utf-8')
        expect(await page.text()).toContain('fixture explorer')
        expect((await fetch(`${url}/app.js`)).headers.get('content-type')).toBe('text/javascript; charset=utf-8')
        expect((await fetch(`${url}/blob.wasm`)).headers.get('content-type')).toBe('application/wasm')
        expect((await fetch(`${url}/../package.json`)).status).not.toBe(200)
    })

    test('by default it prints one `ready` event: the address, the data directory and the database server behind it; nothing on stderr', async () => {
        const { cli, data } = await explorer([], { human: false })
        const info = JSON.parse(cli.stdout().split('\n')[0]!)
        expect(Object.keys(info).sort()).toEqual(['data', 'event', 'server', 'url'])
        expect(info.event).toBe('ready')
        expect(cli.stderr()).toBe('')
        expect(info.data).toBe(data)
        expect(info.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        expect(info.server).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        expect(info.server).not.toBe(info.url)
    })

    test('runs the bundled server on this data directory, on a port of its own, with no query time limit; the page is told what it is', async () => {
        const { cli, url, data } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        expect(recorded(data).argv).toEqual(['start', '--data', data, '--port', '0', '--query-time-limit', '0'])
        const config = await (await fetch(`${url}/explorer-config.json`)).json()
        expect(config).toEqual({
            backend: 'native',
            ui: 'cli',
            local: { label: `local server · ${path.basename(data)}`, detail: data },
            remote: { base: '/_zega/remote', graphs: '/_zega/graphs' },
        })
    })

    test('what is not a file of the page goes to the database: method, path, query, type and body, and the answer comes back', async () => {
        const { cli, url, data } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        const answer = await fetch(`${url}/zql?x=1`, {
            method: 'POST',
            headers: { 'content-type': 'application/json', origin: url, cookie: 'session=secret' },
            body: JSON.stringify({ schema: 's', query: '{ A }' }),
        })
        expect(answer.status).toBe(200)
        expect(answer.headers.get('x-from')).toBe('fake-zega-server')
        expect(await answer.json()).toEqual({ ok: true, result: { echoed: '{ A }' } })
        expect((await (await fetch(`${url}/graph`, { headers: { accept: 'application/json' } })).json())).toEqual({ ok: true, result: { nodes: [], rels: [] } })
        expect((await fetch(`${url}/graph`, { method: 'DELETE' })).status).toBe(200)
        const seen = recorded(data).requests
        expect(seen[0]).toMatchObject({ method: 'POST', target: '/zql?x=1', contentType: 'application/json', body: JSON.stringify({ schema: 's', query: '{ A }' }) })
        expect(seen.map((r) => `${r.method} ${r.target}`)).toEqual(['POST /zql?x=1', 'GET /graph', 'DELETE /graph'])
        // Nothing of the page's own identity reaches the database.
        expect(seen[0]!.cookie).toBeNull()
        expect(seen[0]!.origin).toBeNull()
        expect((await fetch(`${url}/nothing-here`)).status).toBe(404)
    })

    test('answers only at its own address: a rebinding Host and a foreign Origin on a write are refused, and reach nothing', async () => {
        const { cli, url, data } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        const port = new URL(url).port
        const rebinding = await fetch(`${url}/graph`, { headers: { host: `evil.example:${port}` } })
        expect(rebinding.status).toBe(403)
        const foreign = await fetch(`${url}/graph`, { method: 'DELETE', headers: { origin: 'https://evil.example' } })
        expect(foreign.status).toBe(403)
        expect(await foreign.text()).toContain('a page of another origin may not change this database')
        // Its own origin, and a request that names no origin, are served.
        expect((await fetch(`${url}/graph`, { headers: { host: `localhost:${port}` } })).status).toBe(200)
        expect((await fetch(`${url}/graph`, { method: 'DELETE', headers: { origin: url } })).status).toBe(200)
        expect(recorded(data).requests.map((r) => `${r.method} ${r.target}`)).toEqual(['GET /graph', 'DELETE /graph'])
    })

    test('Ctrl-C (SIGTERM) stops the explorer and the database server it started, and exits 0', async () => {
        const { cli, url, data } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        cli.signal('SIGTERM')
        expect(await cli.exited).toBe(0)
        expect(recorded(data).stopped).toBe(true)
        // The database server is asked the way its own Ctrl-C asks (SIGINT: the real one shuts down cleanly on it, and dies at once of SIGTERM).
        expect(fs.readFileSync(path.join(data, 'stopped'), 'utf8')).toBe('SIGINT\n')
        await expect(fetch(url)).rejects.toThrow()
    })

    test('a database that cannot start says why and exits 1, with nothing listening', async () => {
        const cli = startCli(['explorer', '--port', '0', '--server', FAKE_SERVER, '--data', dataDir('locked'), '--assets', FIXTURE_ASSETS], { cwd: work, home })
        expect(await cli.exited).toBe(1)
        expect(cli.stderr()).toContain('the database server (zega-server start) exited with 1 before it was ready')
        expect(cli.stderr()).toContain('data directory is locked')
        expect(cli.stdout()).toBe('')
    })

    test('a port that is taken is named, and the database server it had started is stopped again', async () => {
        const taken = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('') })
        try {
            const data = dataDir()
            const cli = startCli(['explorer', '--port', String(taken.port), '--server', FAKE_SERVER, '--data', data, '--assets', FIXTURE_ASSETS], { cwd: work, home })
            expect(await cli.exited).toBe(1)
            expect(cli.stderr()).toContain(`port ${taken.port} is already in use`)
            expect(recorded(data).stopped).toBe(true)
        } finally {
            taken.stop(true)
        }
    })

    test('with an empty lookup root it says which package, and never looks on PATH', async () => {
        const emptyRoot = path.join(home, 'empty-lookup-root')
        expect(() => resolveServer({ platform: 'linux', arch: 'x64', roots: [emptyRoot] })).toThrow(/is not installed: it is the npm package @zegadb\/server-linux-x64@/)
        expect(() => resolveServer({ platform: 'linux', arch: 'arm64', roots: [emptyRoot] })).toThrow(/there is no zega-server for linux-arm64: /)
        // On a host zega publishes no server for (linux-arm64 CI), the command says that instead.
        const missing = serverPackage(process.platform, process.arch) === null ? /there is no zega-server for / : /is not installed: it is the npm package @zegadb\/server-/
        await expect(runExplorer(undefined, {
            human: false,
            port: 0,
            assets: FIXTURE_ASSETS,
            data: dataDir('missing-server'),
            serverRoots: [emptyRoot],
        })).rejects.toThrow(missing)
    })
})

describe.skipIf(!posix)('zega explorer: a cloud graph, reached through the command', () => {
    const KEY = 'zk_abcdefghijklmnopqrstuvwxyz234567'

    test('the graphs of the signed-in account are listed by name, from the API, with the stored token', async () => {
        const api = fakeApi({ 'GET /v1/graphs': { body: { graphs: [graph, { ...graph, id: 'g0bbbbbbbbbbbbbbbb', name: 'paused', status: 'suspended' }] } } })
        try {
            const { cli, url } = await explorer(['--api', api.url, '--token-file', tokenFile])
            await cli.until('Ctrl-C to stop', 'stderr')
            const answer = await (await fetch(`${url}/_zega/graphs`)).json()
            expect(answer).toEqual({ ok: true, graphs: [{ id: graph.id, name: graph.name, project: graph.projectName }] })
            expect(api.seen.map((r) => `${r.method} ${r.pathname} ${r.authorization}`)).toEqual([`GET /v1/graphs Bearer ${TOKEN}`])
        } finally {
            api.stop()
        }
    })

    test('with no token the list says to log in, and the page still works', async () => {
        const { cli, url } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        const answer = await fetch(`${url}/_zega/graphs`)
        expect(answer.status).toBe(401)
        expect(((await answer.json()) as { error: string }).error).toContain('zega login')
        expect((await fetch(url)).status).toBe(200)
    })

    test('requests for a graph go to its own address (from the API), with only the key the page sent', async () => {
        const host = fakeApi({ 'GET /graph': { body: { ok: true, result: { nodes: [{ id: 1 }], rels: [] } } }, 'POST /zql': { body: { ok: true, result: [] } } })
        const api = fakeApi({ 'GET /v1/graphs': { body: { graphs: [{ ...graph, hostUrl: host.url }] } } })
        try {
            const { cli, url, data } = await explorer(['--api', api.url, '--token-file', tokenFile])
            await cli.until('Ctrl-C to stop', 'stderr')
            await fetch(`${url}/_zega/graphs`) // the page lists the graphs before it connects
            const answer = await fetch(`${url}/_zega/remote/${graph.id}/graph`, {
                headers: { authorization: `Bearer ${KEY}`, accept: 'application/json', cookie: 'a=b', origin: url },
            })
            expect(await answer.json()).toEqual({ ok: true, result: { nodes: [{ id: 1 }], rels: [] } })
            const write = await fetch(`${url}/_zega/remote/${graph.id}/zql?q=1`, {
                method: 'POST',
                headers: { authorization: `Bearer ${KEY}`, 'content-type': 'application/json' },
                body: '{"query":"x"}',
            })
            expect(write.status).toBe(200)
            expect(host.seen.map((r) => `${r.method} ${r.target}`)).toEqual(['GET /graph', 'POST /zql?q=1'])
            expect(host.seen[0]!.authorization).toBe(`Bearer ${KEY}`)
            expect(host.seen[1]!.body).toBe('{"query":"x"}')
            // The account's token went to the API and nowhere else; the graph key went to the graph and nowhere else.
            expect(host.seen.map((r) => r.authorization)).not.toContain(`Bearer ${TOKEN}`)
            expect(api.seen.map((r) => r.authorization)).not.toContain(`Bearer ${KEY}`)
            expect(recorded(data).requests).toEqual([])
            // The log of the command holds neither.
            expect(cli.stdout() + cli.stderr()).not.toContain(KEY)
            expect(cli.stdout() + cli.stderr()).not.toContain(TOKEN)
        } finally {
            host.stop()
            api.stop()
        }
    })

    test('a graph id that is not one, and a graph that cannot be reached, are refused plainly', async () => {
        const { cli, url } = await explorer()
        await cli.until('Ctrl-C to stop', 'stderr')
        const bad = await fetch(`${url}/_zega/remote/NOT_AN_ID/graph`)
        expect(bad.status).toBe(400)
        expect(((await bad.json()) as { error: string }).error).toContain('a graph id is lowercase letters, digits and dashes')
        const foreign = await fetch(`${url}/_zega/remote/${graph.id}/graph`, { method: 'DELETE', headers: { origin: 'https://evil.example' } })
        expect(foreign.status).toBe(403)
    })

    test('an unknown graph address is https://<id>.zegadb.com (the cloud rule), and a failure to reach it is a 502 that names the host', async () => {
        const seen: string[] = []
        const real = globalThis.fetch
        globalThis.fetch = (async (input: Request | string | URL, init?: RequestInit) => {
            const target = String(input)
            if (target.startsWith('https://')) {
                seen.push(`${init?.method ?? 'GET'} ${target}`)
                throw new Error('offline')
            }
            return real(input, init)
        }) as typeof fetch
        const options: ExplorerOptions = {
            port: 0,
            assets: directoryAssets(FIXTURE_ASSETS),
            upstream: 'http://127.0.0.1:1',
            local: { label: 'l', detail: 'd' },
            graphs: async () => null,
            graphHost: async () => null,
        }
        const server = serveExplorer(options)
        try {
            const answer = await real(`http://127.0.0.1:${server.port}/_zega/remote/${graph.id}/zql`, { method: 'POST', body: '{}', headers: { authorization: `Bearer ${KEY}` } })
            expect(answer.status).toBe(502)
            expect(((await answer.json()) as { error: string }).error).toBe(`cannot reach the graph at ${graph.id}.zegadb.com`)
            expect(seen).toEqual([`POST https://${graph.id}.zegadb.com/zql`])
        } finally {
            globalThis.fetch = real
            server.stop(true)
        }
    })
})

describe('zega explorer <graph>: the cloud console address', () => {
    test('prints the dashboard console address of that graph, asked of the API with the token; opens nothing', async () => {
        const api = fakeApi({ [`GET /v1/graphs/${graph.id}`]: { body: { graph } } })
        try {
            const cli = startCli(['explorer', graph.id, '--human', '--api', api.url, '--token-file', tokenFile], { cwd: work, home })
            expect(await cli.exited).toBe(0)
            expect(cli.stdout()).toBe(`https://dashboard.zega.dev/graphs/${graph.id}/console\n`)
            expect(api.seen).toHaveLength(1)
            expect(api.seen[0]).toMatchObject({ method: 'GET', pathname: `/v1/graphs/${graph.id}`, authorization: `Bearer ${TOKEN}` })

            const json = startCli(['explorer', graph.id, '--api', api.url, '--token-file', tokenFile], { cwd: work, home })
            expect(await json.exited).toBe(0)
            expect(JSON.parse(json.stdout())).toEqual({ url: `https://dashboard.zega.dev/graphs/${graph.id}/console` })
        } finally {
            api.stop()
        }
    })

    test('staging has its own dashboard; production and a plain host keep the production one', () => {
        expect(dashboardFor('https://cloud.zega.world')).toBe('https://dashboard.zega.world')
        expect(dashboardFor('https://cloud.zega.dev')).toBe('https://dashboard.zega.dev')
        expect(dashboardFor('http://127.0.0.1:8787')).toBe('https://dashboard.zega.dev')
    })

    test('an unknown graph is the API\'s own answer, and no token says how to log in', async () => {
        const api = fakeApi({})
        try {
            const missing = startCli(['explorer', 'g0zzzzzzzzzzzzzzzz', '--human', '--api', api.url, '--token-file', tokenFile], { cwd: work, home })
            expect(await missing.exited).toBe(1)
            expect(missing.stderr()).toContain('zega: No such route. (code: not_found, HTTP 404)')
            expect(missing.stdout()).toBe('')
            const loggedOut = startCli(['explorer', graph.id, '--human', '--api', api.url], { cwd: work, home })
            expect(await loggedOut.exited).toBe(1)
            expect(loggedOut.stderr()).toContain('not logged in: run `zega login`')
        } finally {
            api.stop()
        }
    })
})
