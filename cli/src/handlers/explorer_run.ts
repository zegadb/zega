import path from 'node:path'

import { openSessionIfAny, type Globals } from '@/handlers/session'
import { directoryAssets, embeddedAssets } from '@/handlers/explorer_assets'
import { serveExplorer } from '@/handlers/explorer_server'
import { LocalFailure } from '@/utils/errors'

export const DEFAULT_PORT = 9343

export type Serving = { url: string; stop(): Promise<void> }

/**
 * The explorer page for the database held by the `zega-server start` at `upstream`: what `zega explorer`
 * (which starts that server) and `zega start --explorer` (which has started it) both serve.
 */
export async function serveDatabaseExplorer(options: {
    port: number
    /** The server's address. */
    upstream: string
    /** The bearer token that server was started to require, if any. */
    token?: string
    /** Its data directory, for the page to name. */
    data: string
    /** A directory of the page to serve instead of the vendored one (development). */
    assets?: string
    globals: Globals
}): Promise<Serving> {
    const assets = options.assets === undefined ? await embeddedAssets() : directoryAssets(options.assets)
    if (assets.get('index.html') === null) {
        throw new LocalFailure(`no explorer page to serve${options.assets === undefined ? '' : ` in ${options.assets}`}: index.html is missing`)
    }
    // The address of each graph the account has, learned when the page lists them, so a request for one goes to it (staging's differ).
    const hosts = new Map<string, string>()
    let web: ReturnType<typeof serveExplorer>
    try {
        web = serveExplorer({
            port: options.port,
            assets,
            upstream: options.upstream,
            upstreamAuthorization: options.token === undefined ? undefined : `Bearer ${options.token}`,
            local: { label: `local server · ${path.basename(options.data)}`, detail: options.data },
            graphs: async () => {
                const session = await openSessionIfAny(options.globals)
                if (session === null) return null
                const { graphs } = (await session.client.get('/v1/graphs')).json
                return graphs
                    .filter((graph) => graph.status === 'active')
                    .map((graph) => {
                        if (graph.hostUrl !== null) hosts.set(graph.id, graph.hostUrl)
                        return { id: graph.id, name: graph.name, project: graph.projectName ?? null }
                    })
            },
            graphHost: async (id) => hosts.get(id) ?? null,
        })
    } catch (error) {
        const taken = (error as NodeJS.ErrnoException).code === 'EADDRINUSE'
        throw new LocalFailure(
            taken
                ? `the explorer's port ${options.port} is already in use: pick another with --port (0 chooses a free one)`
                : `cannot serve the explorer: ${(error as Error).message}`,
        )
    }
    return { url: `http://127.0.0.1:${web.port}`, stop: async () => void (await web.stop(true)) }
}
