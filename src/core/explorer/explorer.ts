import path from 'node:path'

import { openSession, type Globals } from '@/handlers'
import { startLocal } from '@/handlers/explorer_local'
import { DEFAULT_PORT, serveDatabaseExplorer, type Serving } from '@/handlers/explorer_run'
import { resolveServer } from '@/handlers/server_binary'
import { LocalFailure } from '@/utils/errors'
import { DASHBOARD } from '@/utils/constants'
import { json, write } from '@/utils'

export type ExplorerOptions = Globals & {
    /** The explorer's own port (default 9343, as `zega-server explorer` had); 0 asks the OS. */
    port?: number
    /** The database's directory (default ./zega-data). */
    data?: string
    /** Run this zega-server instead of the bundled one (development). */
    server?: string
    /** Serve this directory's build of the page instead of the vendored one (development). */
    assets?: string
}

export { DEFAULT_PORT }
export const DEFAULT_DATA = './zega-data'

/**
 * Where the dashboard is for the API this command talks to: production's, or the same name
 * on the staging domain (`cloud.zega.world` has `dashboard.zega.world`).
 */
export function dashboardFor(api: string): string {
    const host = new URL(api).hostname
    return host.startsWith('cloud.') && host !== 'cloud.zega.dev' ? `https://dashboard.${host.slice('cloud.'.length)}` : DASHBOARD
}

/**
 * `zega explorer`: the explorer page for a local database, served here (it prints the address as a
 * `ready` event, or with `--human` as a plain line, and opens nothing). `zega explorer <graph>`: the address of that cloud graph's console in the dashboard.
 */
export async function explorer(graph: string | undefined, options: ExplorerOptions): Promise<void> {
    if (graph !== undefined) return await consoleUrl(graph, options)
    await serveLocal(options)
}

async function consoleUrl(id: string, options: ExplorerOptions): Promise<void> {
    const session = await openSession(options)
    const { graph } = (await session.client.get('/v1/graphs/{graph}', { path: { graph: id } })).json
    // `/graphs/<id>/console` is the dashboard's address today; its project-first layout redirects it (zegadb/dashboard#44).
    const url = `${dashboardFor(session.api)}/graphs/${encodeURIComponent(graph.id)}/console`
    await write('stdout', options.human === true ? `${url}\n` : json({ url }))
}

async function serveLocal(options: ExplorerOptions): Promise<void> {
    const data = path.resolve(options.data ?? DEFAULT_DATA)
    const binary = resolveServer({ override: options.server })
    const local = await startLocal(binary, data)
    let web: Serving
    try {
        web = await serveDatabaseExplorer({
            port: options.port ?? DEFAULT_PORT,
            upstream: local.url,
            data,
            assets: options.assets,
            globals: options,
        })
    } catch (error) {
        await local.stop()
        throw error
    }

    if (options.human === true) {
        await write('stdout', `${web.url}\n`)
        await write('stderr', `explorer for ${data} (database server at ${local.url}); Ctrl-C to stop\n`)
    } else {
        await write('stdout', json({ event: 'ready', url: web.url, data, server: local.url }))
    }

    const reason = await Promise.race([
        new Promise<'signal'>((resolve) => {
            process.once('SIGINT', () => resolve('signal'))
            process.once('SIGTERM', () => resolve('signal'))
        }),
        local.exited.then((code) => `the database server stopped (exit ${code})` as const),
    ])
    await web.stop()
    await local.stop()
    if (reason !== 'signal') throw new LocalFailure(reason)
}
