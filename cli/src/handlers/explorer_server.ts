/**
 * The explorer the `zega` command serves: the page (vendored from zegadb/zega), the database
 * it works on, and, when asked, a cloud graph.
 *
 * The page is zegadb/zega's explorer in its "native" mode: it expects the database's own HTTP
 * routes (`/zql`, `/graph`, ...) on the same origin and asks `/explorer-config.json` what it is.
 * Here the database is a `zega-server start` this command runs (a child process that owns the
 * data directory), and everything that is not a file of the page is passed on to it, so writes
 * go to the real engine and a local database is persistent. A cloud graph is reached the same
 * way: the page's requests to `/_zega/remote/<graph>/...` are passed on to the graph's own
 * address with the graph key the page sends, which is why the cloud's CORS rule (it allows one
 * origin, https://explorer.zega.dev) does not matter and why nothing is kept here: no key, no
 * body, no log of either.
 *
 * It listens on 127.0.0.1 only. A page of some other website must not be able to drive a database
 * through the visitor's own machine, so the Host must be this server's (DNS rebinding) and a
 * request that can change anything must carry no Origin or this server's own.
 */
import type { Assets } from '@/handlers/explorer_assets'

export type ExplorerOptions = {
    port: number
    assets: Assets
    /** The `zega-server start` that holds the database: `http://127.0.0.1:<port>`. */
    upstream: string
    /** `Bearer <token>` when that server was started with a token file: the page never sees it. */
    upstreamAuthorization?: string
    /** What the page calls the database, and its longer name (a tooltip). */
    local: { label: string; detail: string }
    /** The graphs of the signed-in account, by name; null when there is no token. */
    graphs: () => Promise<{ id: string; name: string; project: string | null }[] | null>
    /** The address of a cloud graph: from the API when signed in, else null (the default is used). */
    graphHost: (id: string) => Promise<string | null>
}

export const REMOTE_BASE = '/_zega/remote'
export const GRAPHS_PATH = '/_zega/graphs'
const GRAPH_ID = /^[a-z0-9-]{1,40}$/

/** Headers that belong to one connection, not to the message. */
const HOP = new Set(['connection', 'keep-alive', 'transfer-encoding', 'content-encoding', 'content-length', 'upgrade', 'proxy-authorization', 'te', 'trailer'])

const text = (status: number, body: string): Response => new Response(body + '\n', { status, headers: { 'content-type': 'text/plain; charset=utf-8' } })

/** Pass a response on without what belongs to the connection or to the cloud's own page (cookies). */
function passOn(upstream: Response): Response {
    const headers = new Headers()
    upstream.headers.forEach((value, name) => {
        if (!HOP.has(name) && name !== 'set-cookie' && !name.startsWith('access-control-')) headers.set(name, value)
    })
    headers.set('cache-control', 'no-store')
    return new Response(upstream.body, { status: upstream.status, headers })
}

export function serveExplorer(options: ExplorerOptions): ReturnType<typeof Bun.serve> {
    const config = {
        backend: 'native',
        ui: 'cli',
        local: options.local,
        remote: { base: REMOTE_BASE, graphs: GRAPHS_PATH },
    }
    const server: ReturnType<typeof Bun.serve> = Bun.serve({
        hostname: '127.0.0.1',
        port: options.port,
        // A big import is one request: never cut it at the default limit.
        maxRequestBodySize: 4 * 1024 ** 3,
        idleTimeout: 120,
        async fetch(request) {
            const own = `127.0.0.1:${server.port}`
            const host = request.headers.get('host')
            if (host !== own && host !== `localhost:${server.port}`) return text(403, 'zega: this explorer answers only at its own address')
            const origin = request.headers.get('origin')
            const safe = request.method === 'GET' || request.method === 'HEAD'
            if (!safe && origin !== null && origin !== `http://${own}` && origin !== `http://localhost:${server.port}`) {
                return text(403, 'zega: a page of another origin may not change this database')
            }
            const url = new URL(request.url)
            const pathname = url.pathname

            if (pathname === '/explorer-config.json') return Response.json(config, { headers: { 'cache-control': 'no-store' } })

            if (pathname === GRAPHS_PATH) {
                const graphs = await options.graphs().catch(() => null)
                if (graphs === null) return Response.json({ ok: false, error: 'not signed in: run `zega login`' }, { status: 401 })
                return Response.json({ ok: true, graphs }, { headers: { 'cache-control': 'no-store' } })
            }

            if (pathname.startsWith(`${REMOTE_BASE}/`)) return await remote(request, url, options)

            if (safe) {
                const asset = options.assets.get(pathname === '/' ? 'index.html' : decodeURIComponent(pathname.slice(1)))
                if (asset !== null) {
                    return new Response(request.method === 'HEAD' ? null : asset, {
                        headers: { 'content-type': asset.type, 'cache-control': 'no-cache', 'x-content-type-options': 'nosniff' },
                    })
                }
            }
            return await local(request, url, options)
        },
    })
    return server
}

/** The page's request to the database: the same request, to the `zega-server start` behind this. */
async function local(request: Request, url: URL, options: ExplorerOptions): Promise<Response> {
    const headers = new Headers()
    for (const name of ['accept', 'content-type']) {
        const value = request.headers.get(name)
        if (value !== null) headers.set(name, value)
    }
    if (options.upstreamAuthorization !== undefined) headers.set('authorization', options.upstreamAuthorization)
    try {
        const upstream = await fetch(options.upstream + url.pathname + url.search, {
            method: request.method,
            headers,
            body: request.method === 'GET' || request.method === 'HEAD' ? undefined : request.body,
            redirect: 'manual',
        })
        return passOn(upstream)
    } catch {
        return Response.json({ ok: false, error: 'the local database server is not answering' }, { status: 502 })
    }
}

/** The page's request to a cloud graph: `/_zega/remote/<graph>/<path>` becomes `<the graph's address>/<path>`, with the graph key the page sent. */
async function remote(request: Request, url: URL, options: ExplorerOptions): Promise<Response> {
    const rest = url.pathname.slice(REMOTE_BASE.length + 1)
    const slash = rest.indexOf('/')
    const id = slash === -1 ? rest : rest.slice(0, slash)
    const path = slash === -1 ? '/' : rest.slice(slash)
    if (!GRAPH_ID.test(id)) return Response.json({ ok: false, error: 'a graph id is lowercase letters, digits and dashes' }, { status: 400 })
    const host = (await options.graphHost(id).catch(() => null)) ?? `https://${id}.zegadb.com`
    // Only these cross: no cookies, no origin, nothing of the page's own.
    const headers = new Headers()
    for (const name of ['authorization', 'accept', 'content-type']) {
        const value = request.headers.get(name)
        if (value !== null) headers.set(name, value)
    }
    try {
        const upstream = await fetch(host.replace(/\/$/, '') + path + url.search, {
            method: request.method,
            headers,
            body: request.method === 'GET' || request.method === 'HEAD' ? undefined : request.body,
            redirect: 'manual',
        })
        return passOn(upstream)
    } catch {
        return Response.json({ ok: false, error: `cannot reach the graph at ${new URL(host).host}` }, { status: 502 })
    }
}
