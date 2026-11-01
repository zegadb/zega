import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FAKE_QUERY_SERVER, callTool, connectMcp, newHome, toolNames } from './mcp_helpers'
import { startCli } from './cli_helpers'
import { CLIENTS, connect } from '../src/mcp/clients'
import { FIXTURES, FUNCTION_ID, TOKEN, account, deployedFunction, fakeApi, graph, project, regionsBody, remove, tokenInfo } from './helpers'

const MADE_KEY = 'zk_abcdefghijklmnopqrstuvwxyz234567'
const GIVEN_KEY = 'zk_' + 'b'.repeat(32)
const STORED_SCHEMA = 'schema { type Person { name: String } }'

/** The graph's own address: records what reaches it, answers as a real graph does (QUERY runs reads only). */
function fakeGraphHost() {
    const seen: { method: string; path: string; authorization: string | null; body: string }[] = []
    const server = Bun.serve({
        hostname: '127.0.0.1',
        port: 0,
        async fetch(request) {
            const url = new URL(request.url)
            const body = request.method === 'GET' ? '' : await request.text()
            seen.push({ method: request.method, path: url.pathname, authorization: request.headers.get('authorization'), body })
            if (url.pathname === '/schema') return Response.json({ ok: true, result: { schema: STORED_SCHEMA, updatedAt: null } })
            if (url.pathname === '/zql') {
                const sent = JSON.parse(body) as { query: string }
                if (request.method === 'QUERY' && /\bmutation\b/.test(sent.query)) {
                    return Response.json({ ok: false, error: 'QUERY /zql runs reads only', code: 'not_a_read' }, { status: 422 })
                }
                return Response.json({ ok: true, result: { via: request.method, rows: [{ name: 'Ada' }] } })
            }
            return Response.json({ ok: false, error: 'not found' }, { status: 404 })
        },
    })
    return { seen, url: `http://127.0.0.1:${server.port}`, stop: () => server.stop(true) }
}

type Scope = 'read' | 'manage'

/** A fake Zega Cloud API whose token has this scope, with a graph at `hostUrl`. */
function cloud(scope: Scope, hostUrl: string) {
    const info = { ...tokenInfo, scope }
    const here = { ...graph, hostUrl }
    return fakeApi({
        'GET /v1/whoami': { body: { account, token: info } },
        'GET /v1/projects': { body: { projects: [project] } },
        'GET /v1/regions': { body: regionsBody },
        'GET /v1/usage': { body: { month: '2026-10', graphs: [] } },
        'GET /v1/graphs': { body: { graphs: [here] } },
        [`GET /v1/graphs/${graph.id}`]: { body: { graph: here } },
        [`GET /v1/graphs/${graph.id}/usage`]: { body: { graph: graph.id, usage: { queries: 3 } } },
        [`POST /v1/graphs/${graph.id}/keys`]: {
            status: 201,
            body: { key: { id: 'key_1', name: 'zega mcp', prefix: 'zk_abcd', createdAt: '2026-10-03T00:00:00.000Z', secret: MADE_KEY }, shownOnce: true },
        },
        [`DELETE /v1/graphs/${graph.id}/keys/key_1`]: { body: { revoked: 'key_1' } },
        'GET /v1/functions': { body: { functions: [deployedFunction] } },
        [`GET /v1/functions/${FUNCTION_ID}/code`]: { body: { code: 'export default {}' } },
        [`PUT /v1/functions/${FUNCTION_ID}/code`]: { body: { function: deployedFunction } },
        // A buggy or hostile answer that repeats the credential it was given.
        'GET /v1/buckets': { status: 500, body: { ok: false, code: 'oops', error: `internal error for ${TOKEN} and ${MADE_KEY}` } },
    })
}

const WRITE_TOOLS = ['zega_function_deploy', 'zega_graph_mutate', 'zega_local_mutate', 'zega_local_start', 'zega_local_stop']
const DOCS = ['zega_docs_read', 'zega_docs_search']
const ACCOUNT = ['zega_projects', 'zega_regions', 'zega_usage', 'zega_whoami']
const GRAPHS_READ = ['zega_graph_query', 'zega_graph_schema', 'zega_graph_stats', 'zega_graphs']
const FUNCTIONS_READ = ['zega_function_code', 'zega_function_logs', 'zega_functions']
const STORAGE = ['zega_bucket_objects', 'zega_buckets']
const LOCAL_READ = ['zega_local_query', 'zega_local_status']

const home = newHome()
afterAll(() => remove(home))
const hello = path.join(FIXTURES, 'hello')

describe('mcp config formats', () => {
    test('prints the documented command or native config format for every client, including read-only and spaced token paths', async () => {
        expect(CLIENTS).toEqual(['claude-code', 'codex', 'cursor', 'vscode', 'windsurf', 'zed'])
        const common = { name: 'zega', groups: ['account', 'graphs'] }
        const configured = Object.fromEntries(CLIENTS.map((client) => [client, connect({ client, ...common }).snippets]))
        expect(configured['claude-code']![0]!.text).toBe('claude mcp add --scope project zega -- zega mcp --groups account,graphs')
        expect(JSON.parse(configured['claude-code']![2]!.text)).toEqual({ mcpServers: { zega: { type: 'stdio', command: 'zega', args: ['mcp', '--groups', 'account,graphs'] } } })
        expect(configured.codex![0]!.text).toBe('codex mcp add zega -- zega mcp --groups account,graphs')
        expect(configured.codex![1]!.text).toContain('[mcp_servers.zega]\ncommand = "zega"\nargs = ["mcp", "--groups", "account,graphs"]')
        expect(JSON.parse(configured.cursor![0]!.text)).toEqual({ mcpServers: { zega: { command: 'zega', args: ['mcp', '--groups', 'account,graphs'] } } })
        expect(JSON.parse(configured.vscode![0]!.text)).toEqual({ servers: { zega: { type: 'stdio', command: 'zega', args: ['mcp', '--groups', 'account,graphs'] } } })
        expect(JSON.parse(configured.vscode![1]!.text.match(/\{.*\}/)![0]!)).toEqual({ name: 'zega', command: 'zega', args: ['mcp', '--groups', 'account,graphs'] })
        expect(JSON.parse(configured.windsurf![0]!.text)).toEqual({ mcpServers: { zega: { command: 'zega', args: ['mcp', '--groups', 'account,graphs'] } } })
        expect(JSON.parse(configured.zed![0]!.text)).toEqual({ context_servers: { zega: { command: 'zega', args: ['mcp', '--groups', 'account,graphs'], env: {} } } })

        for (const client of CLIENTS) {
            const tokenFile = path.join(home, 'Ada Lovelace', 'zega token')
            const [first] = connect({ client, ...common, readOnly: true, tokenFile }).snippets
            expect(first!.text).toContain('--read-only')
            expect(first!.text).toContain(tokenFile)
            for (const snippet of connect({ client, ...common, readOnly: true, tokenFile }).snippets) {
                expect(snippet.text).not.toContain('zk_')
            }
            const cli = startCli(
                ['mcp', 'config', '--client', client, '--groups', 'account,graphs', '--read-only', '--token-file', tokenFile],
                { cwd: home, home },
            )
            expect(await cli.exited).toBe(0)
            expect(JSON.parse(cli.stdout())).toEqual(connect({ client, ...common, readOnly: true, tokenFile }))
            expect(cli.stdout()).not.toContain('zk_')
        }
    }, 30_000)
})

describe('which tools a server offers', () => {
    test('by default every group except storage, with the write tools for a manage token', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp([], { home, api: api.url })
        try {
            expect(await toolNames(mcp)).toEqual([...ACCOUNT, ...DOCS, ...FUNCTIONS_READ, ...GRAPHS_READ, ...LOCAL_READ, ...WRITE_TOOLS].sort())
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })

    test('--groups chooses the groups: one group offers only its tools, and storage is there when asked for', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        try {
            const docs = await connectMcp(['--groups', 'docs'], { home, api: api.url })
            expect(await toolNames(docs)).toEqual(DOCS)
            await docs.close()
            const some = await connectMcp(['--groups', 'account,graphs'], { home, api: api.url })
            expect(await toolNames(some)).toEqual([...ACCOUNT, ...GRAPHS_READ, 'zega_graph_mutate'].sort())
            await some.close()
            const storage = await connectMcp(['--groups', 'storage', '--read-only'], { home, api: api.url })
            expect(await toolNames(storage)).toEqual(STORAGE)
            await storage.close()
            const all = await connectMcp(['--groups', 'all'], { home, api: api.url })
            expect(await toolNames(all)).toEqual([...ACCOUNT, ...DOCS, ...FUNCTIONS_READ, ...GRAPHS_READ, ...LOCAL_READ, ...STORAGE, ...WRITE_TOOLS].sort())
            await all.close()
        } finally {
            api.stop()
            graphHost.stop()
        }
    })

    test('--read-only offers no tool that changes anything, whatever the token may do', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp(['--read-only', '--groups', 'all'], { home, api: api.url })
        try {
            const names = await toolNames(mcp)
            for (const write of WRITE_TOOLS) expect(names).not.toContain(write)
            expect(names).toEqual([...ACCOUNT, ...DOCS, ...FUNCTIONS_READ, ...GRAPHS_READ, ...LOCAL_READ, ...STORAGE].sort())
            // And a client that asks for one anyway gets nothing done.
            const refused = await mcp.client.callTool({ name: 'zega_graph_mutate', arguments: { graph: graph.id, query: 'mutation { }', document: true } }).catch((error: Error) => error)
            expect(refused instanceof Error || (refused as { isError?: boolean }).isError === true).toBe(true)
            expect(graphHost.seen).toEqual([])
            expect(api.seen.filter((r) => r.method !== 'GET')).toEqual([])
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })

    test('a read token offers only tools that read, with no --read-only given', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('read', graphHost.url)
        const mcp = await connectMcp(['--groups', 'all'], { home, api: api.url })
        try {
            const names = await toolNames(mcp)
            for (const write of WRITE_TOOLS) expect(names).not.toContain(write)
            expect(names).toEqual([...ACCOUNT, ...DOCS, ...FUNCTIONS_READ, ...GRAPHS_READ, ...LOCAL_READ, ...STORAGE].sort())
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })

    test('the docs and local groups need no login at all; a cloud group without one stops at the start, with the usual error', async () => {
        const offline = await connectMcp(['--groups', 'docs,local'], { home, api: 'http://127.0.0.1:9' })
        try {
            expect(await toolNames(offline)).toEqual([...DOCS, ...LOCAL_READ, 'zega_local_mutate', 'zega_local_start', 'zega_local_stop'].sort())
        } finally {
            await offline.close()
        }
        await expect(connectMcp(['--groups', 'account'], { home, api: 'http://127.0.0.1:9' })).rejects.toThrow()
    })
})

describe('the read tools call the same commands as the CLI', () => {
    test('zega_projects answers what `zega projects` prints; the token is the one given, as a bearer', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('read', graphHost.url)
        const mcp = await connectMcp([], { home, api: api.url })
        try {
            const answer = await callTool(mcp, 'zega_projects')
            expect(answer.isError).toBe(false)
            expect(JSON.parse(answer.text)).toEqual({ projects: [project] })
            const request = api.seen.find((r) => r.pathname === '/v1/projects')!
            expect(request.authorization).toBe(`Bearer ${TOKEN}`)
            const graphs = JSON.parse((await callTool(mcp, 'zega_graphs', { id: graph.id })).text)
            expect(graphs.graph.id).toBe(graph.id)
            const logs = await callTool(mcp, 'zega_regions')
            expect(JSON.parse(logs.text)).toEqual(regionsBody)
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })

    test('an API error is a tool error with the CLI\'s own code, message and hint', async () => {
        const api = fakeApi({ 'GET /v1/whoami': { body: { account, token: tokenInfo } } })
        const mcp = await connectMcp(['--groups', 'account'], { home, api: api.url })
        try {
            const answer = await callTool(mcp, 'zega_projects', { id: 'nope' })
            expect(answer.isError).toBe(true)
            expect(JSON.parse(answer.text).error.code).toBe('not_found')
        } finally {
            await mcp.close()
            api.stop()
        }
    })

    test('the docs are searched offline: a query finds the page that answers it', async () => {
        const mcp = await connectMcp(['--groups', 'docs'], { home, api: 'http://127.0.0.1:9' })
        try {
            const found = JSON.parse((await callTool(mcp, 'zega_docs_search', { query: 'mutation', limit: 3 })).text) as { results: { page: string; heading: string }[] }
            expect(found.results.length).toBeGreaterThan(0)
            expect(found.results[0]!.page).toBe('mutation')
            const pages = JSON.parse((await callTool(mcp, 'zega_docs_read')).text) as { pages: { page: string }[] }
            expect(pages.pages.map((p) => p.page)).toContain('schema')
            const page = JSON.parse((await callTool(mcp, 'zega_docs_read', { page: 'schema' })).text) as { text: string }
            expect(page.text).toContain('schema')
            expect((await callTool(mcp, 'zega_docs_read', { page: '../package' })).isError).toBe(true)
            expect((await callTool(mcp, 'zega_docs_read', { page: 'constructor' })).isError).toBe(true)
        } finally {
            await mcp.close()
        }
    })
})

describe('graph queries need a graph key, which is never shown', () => {
    test('a manage token makes a key for the session, queries with QUERY (reads only), and revokes the key when the server stops', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp([], { home, api: api.url })
        const outputs: string[] = []
        try {
            const read = await callTool(mcp, 'zega_graph_query', { graph: graph.id, query: 'query { Person { name } }' })
            outputs.push(read.text)
            expect(read.isError).toBe(false)
            expect(JSON.parse(read.text)).toEqual({ via: 'QUERY', rows: [{ name: 'Ada' }] })
            // The API token went to the API only; the key went to the graph only, as the bearer.
            expect(graphHost.seen.map((r) => [r.method, r.path, r.authorization])).toEqual([
                ['GET', '/schema', `Bearer ${MADE_KEY}`],
                ['QUERY', '/zql', `Bearer ${MADE_KEY}`],
            ])
            expect(JSON.parse(graphHost.seen[1]!.body)).toEqual({ schema: STORED_SCHEMA, query: 'query { Person { name } }', document: false })
            expect(api.seen.some((r) => r.method === 'POST' && r.pathname === `/v1/graphs/${graph.id}/keys`)).toBe(true)
            expect(api.seen.filter((r) => r.pathname === '/v1/graphs' || r.pathname === `/v1/graphs/${graph.id}`).every((r) => r.authorization === `Bearer ${TOKEN}`)).toBe(true)

            // A mutation through the read tool is refused by the graph itself.
            const refused = await callTool(mcp, 'zega_graph_query', { graph: graph.id, query: 'mutation { Person { name: "x" } }', document: true })
            outputs.push(refused.text)
            expect(refused.isError).toBe(true)
            expect(JSON.parse(refused.text).error.code).toBe('not_a_read')

            const written = await callTool(mcp, 'zega_graph_mutate', { graph: graph.id, query: 'mutation { }', document: true })
            outputs.push(written.text)
            expect(JSON.parse(written.text).via).toBe('POST')
            expect(graphHost.seen.at(-1)!.method).toBe('POST')

            // One key for the session, not one per call.
            expect(api.seen.filter((r) => r.method === 'POST')).toHaveLength(1)
            expect(api.seen.some((r) => r.method === 'DELETE')).toBe(false)
        } finally {
            await mcp.close()
        }
        for (let i = 0; i < 100 && !api.seen.some((r) => r.method === 'DELETE'); i++) await Bun.sleep(50)
        expect(api.seen.find((r) => r.method === 'DELETE')).toMatchObject({ pathname: `/v1/graphs/${graph.id}/keys/key_1`, authorization: `Bearer ${TOKEN}` })
        for (const text of outputs) expect(text).not.toContain(MADE_KEY)
        api.stop()
        graphHost.stop()
    })

    test('under --read-only no key is made: the tool says what to do, and nothing was sent to the API or the graph', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp(['--read-only'], { home, api: api.url })
        try {
            const answer = await callTool(mcp, 'zega_graph_query', { graph: graph.id, query: 'query { }', document: true })
            expect(answer.isError).toBe(true)
            const error = JSON.parse(answer.text).error
            expect(error.code).toBe('key_unavailable')
            expect(error.message).toContain('--graph-key-file')
            expect(api.seen.filter((r) => r.method === 'POST')).toEqual([])
            expect(graphHost.seen).toEqual([])
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })

    test('a key the person gives (--graph-key-file, or ZEGA_GRAPH_KEY in .env) is used, also with a read token, and none is made', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('read', graphHost.url)
        const keyFile = path.join(home, 'graph-key')
        fs.writeFileSync(keyFile, GIVEN_KEY + '\n', { mode: 0o600 })
        const mcp = await connectMcp(['--groups', 'graphs', '--graph-key-file', keyFile], { home, api: api.url })
        try {
            const answer = await callTool(mcp, 'zega_graph_query', { graph: graph.id, query: 'query { }', document: true })
            expect(JSON.parse(answer.text).via).toBe('QUERY')
            expect(graphHost.seen.at(-1)!.authorization).toBe(`Bearer ${GIVEN_KEY}`)
            expect(answer.text).not.toContain(GIVEN_KEY)
        } finally {
            await mcp.close()
        }
        const project_ = path.join(home, 'with-dev-vars')
        fs.mkdirSync(project_)
        fs.writeFileSync(path.join(project_, '.env'), `ZEGA_GRAPH_KEY=${GIVEN_KEY}\nZEGA_GRAPH_URL=https://${graph.id}.zegadb.com\n`)
        const viaVars = await connectMcp(['--groups', 'graphs'], { home, cwd: project_, api: api.url })
        try {
            const answer = await callTool(viaVars, 'zega_graph_schema', { graph: graph.id })
            expect(JSON.parse(answer.text).schema).toBe(STORED_SCHEMA)
            expect(graphHost.seen.at(-1)!.authorization).toBe(`Bearer ${GIVEN_KEY}`)
            expect(api.seen.filter((r) => r.method === 'POST')).toEqual([])
        } finally {
            await viaVars.close()
            api.stop()
            graphHost.stop()
        }
    })
})

describe('the token never appears in what a tool answers', () => {
    test('not in whoami (the API\'s prefix is dropped too), not in a tool error that repeats it, not in a made key\'s answer', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp(['--groups', 'all'], { home, api: api.url })
        const everything: string[] = []
        try {
            for (const { name } of (await mcp.client.listTools()).tools) {
                const args: Record<string, unknown> = { graph: graph.id, id: FUNCTION_ID, bucket: 'b0cccccccccccccccc', query: 'query { }', document: true, page: 'schema' }
                if (name === 'zega_local_stop') continue
                const answer = await callTool(mcp, name, name === 'zega_docs_search' ? { query: 'zc_test' } : name === 'zega_buckets' ? { id: undefined } : args)
                everything.push(answer.text)
            }
            const who = await callTool(mcp, 'zega_whoami')
            everything.push(who.text)
            expect(JSON.parse(who.text).token).toEqual({ ...tokenInfo, scope: 'manage', prefix: undefined })
            const leaked = await callTool(mcp, 'zega_buckets')
            expect(leaked.isError).toBe(true)
            expect(leaked.text).toContain('internal error for')
            everything.push(leaked.text)
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
        expect(everything.length).toBeGreaterThan(10)
        for (const text of everything) {
            expect(text).not.toContain(TOKEN)
            expect(text).not.toContain(MADE_KEY)
            expect(text).not.toContain(tokenInfo.prefix)
        }
    })

    test('the server\'s own stderr does not have it either', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp([], { home, api: api.url })
        try {
            await callTool(mcp, 'zega_graph_query', { graph: graph.id, query: 'query { }', document: true })
            await callTool(mcp, 'zega_whoami')
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
        expect(mcp.stderr()).not.toContain(TOKEN)
        expect(mcp.stderr()).not.toContain(MADE_KEY)
    })
})

describe('functions', () => {
    test('zega_function_deploy bundles a file of the project and uploads it as `zega function deploy` does; a file outside the project is refused', async () => {
        const graphHost = fakeGraphHost()
        const api = cloud('manage', graphHost.url)
        const mcp = await connectMcp(['--groups', 'functions'], { home, cwd: hello, api: api.url })
        try {
            const done = await callTool(mcp, 'zega_function_deploy', { id: FUNCTION_ID, entry: 'src/index.ts' })
            expect(done.isError).toBe(false)
            expect(JSON.parse(done.text).function.id).toBe(FUNCTION_ID)
            const put = api.seen.find((r) => r.method === 'PUT')!
            expect(put).toMatchObject({ pathname: `/v1/functions/${FUNCTION_ID}/code`, contentType: 'application/javascript', authorization: `Bearer ${TOKEN}` })
            expect(put.body).toContain('hello from workerd')

            const before = api.seen.length
            const outside = await callTool(mcp, 'zega_function_deploy', { id: FUNCTION_ID, entry: '../builtins/src/index.ts' })
            expect(outside.isError).toBe(true)
            expect(JSON.parse(outside.text).error.code).toBe('outside_project')
            expect((await callTool(mcp, 'zega_function_deploy', { id: FUNCTION_ID, entry: '/etc/hosts' })).isError).toBe(true)
            expect(api.seen.length).toBe(before)
        } finally {
            await mcp.close()
            api.stop()
            graphHost.stop()
        }
    })
})

describe('the local group', () => {
    test('start, query (reads only), mutate, status and stop a local server of the project', async () => {
        const work = path.join(home, 'local-project')
        fs.mkdirSync(work)
        const mcp = await connectMcp(['--groups', 'local', '--server', FAKE_QUERY_SERVER], { home, cwd: work, api: 'http://127.0.0.1:9' })
        try {
            const started = JSON.parse((await callTool(mcp, 'zega_local_start', { data: 'db' })).text) as { url: string; data: string }
            expect(started.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
            expect(fs.realpathSync(started.data)).toBe(fs.realpathSync(path.join(work, 'db')))
            expect(JSON.parse((await callTool(mcp, 'zega_local_start', { data: 'db' })).text).alreadyRunning).toBe(true)
            expect((await callTool(mcp, 'zega_local_start', { data: '../elsewhere' })).isError).toBe(true)

            const query = JSON.parse((await callTool(mcp, 'zega_local_query', { query: 'query { }', document: true })).text)
            expect(query).toEqual({ via: 'QUERY', echoed: 'query { }' })
            const refused = await callTool(mcp, 'zega_local_query', { query: 'mutation { }', document: true })
            expect(refused.isError).toBe(true)
            expect(JSON.parse(refused.text).error.code).toBe('not_a_read')
            expect(JSON.parse((await callTool(mcp, 'zega_local_mutate', { query: 'mutation { }', document: true })).text).via).toBe('POST')
            const status = JSON.parse((await callTool(mcp, 'zega_local_status')).text)
            expect(status).toMatchObject({ url: started.url, running: true, stats: { nodes: 1, relationships: 0 } })

            // Only loopback: a graph in the cloud is not a "local server".
            expect((await callTool(mcp, 'zega_local_query', { url: 'https://example.com', query: 'query { }' })).isError).toBe(true)

            await callTool(mcp, 'zega_local_stop')
            expect(fs.readFileSync(path.join(work, 'db', 'stopped'), 'utf8').trim()).toBe('SIGINT')
            expect(JSON.parse((await callTool(mcp, 'zega_local_status', { url: started.url })).text).running).toBe(false)
        } finally {
            await mcp.close()
        }
    })

    test('a server this session started is stopped when the session ends', async () => {
        const work = path.join(home, 'local-project-2')
        fs.mkdirSync(work)
        const mcp = await connectMcp(['--groups', 'local', '--server', FAKE_QUERY_SERVER], { home, cwd: work, api: 'http://127.0.0.1:9' })
        await callTool(mcp, 'zega_local_start', { data: 'db' })
        await mcp.close()
        for (let i = 0; i < 100 && !fs.existsSync(path.join(work, 'db', 'stopped')); i++) await Bun.sleep(50)
        expect(fs.existsSync(path.join(work, 'db', 'stopped'))).toBe(true)
    })
})
