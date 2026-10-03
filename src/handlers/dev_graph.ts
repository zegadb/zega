/**
 * Which graph `zega dev` points a function at, if any.
 *
 * A deployed function has no graph binding: it calls the graph's HTTPS address, which the
 * customer put in a variable, with a graph key (`zk_...`) the customer put in a secret. So
 * local dev sets that one variable, and says which graph it is. The key is a secret like
 * any other and comes from `.dev.vars`; the stored API token never reaches a function (it is
 * used only to ask the API for the graph's address, and goes nowhere else).
 */
import { openSession, type Globals } from '@/handlers/session'
import { LocalFailure } from '@/utils/errors'

/** `zega-server start`'s own default address. */
export const LOCAL_GRAPH_URL = 'http://127.0.0.1:9342'

export const DEFAULT_URL_VAR = 'ZEGA_GRAPH_URL'
export const DEFAULT_KEY_VAR = 'ZEGA_GRAPH_KEY'

export type GraphChoice =
    | { kind: 'none' }
    | { kind: 'local'; url: string; reachable: boolean }
    | { kind: 'cloud'; url: string; id: string; name: string }

export type GraphFlags = { localGraph?: true | string; cloudGraph?: string }

function isLoopback(hostname: string): boolean {
    return hostname === '127.0.0.1' || hostname === 'localhost' || hostname === '[::1]' || hostname === '::1'
}

/** Whether anything answers at the local graph's address (any HTTP status counts: a token-protected server answers 401). */
async function answers(url: string): Promise<boolean> {
    try {
        await fetch(new URL('/health', url), { signal: AbortSignal.timeout(1500) })
        return true
    } catch {
        return false
    }
}

export async function chooseGraph(flags: GraphFlags, globals: Globals): Promise<GraphChoice> {
    if (flags.localGraph !== undefined && flags.cloudGraph !== undefined) {
        throw new LocalFailure('--local-graph and --cloud-graph are alternatives: a function is pointed at one graph')
    }
    if (flags.cloudGraph !== undefined) {
        const { client } = await openSession(globals)
        const { graph } = (await client.get('/v1/graphs/{graph}', { path: { graph: flags.cloudGraph } })).json
        if (graph.hostUrl === null) throw new LocalFailure(`graph ${graph.id} has no address yet (status ${graph.status})`)
        return { kind: 'cloud', url: graph.hostUrl, id: graph.id, name: graph.name }
    }
    if (flags.localGraph !== undefined) {
        const given = flags.localGraph === true ? LOCAL_GRAPH_URL : flags.localGraph
        let url: URL
        try {
            url = new URL(given)
        } catch {
            throw new LocalFailure(`--local-graph: "${given}" is not a URL (the default is ${LOCAL_GRAPH_URL})`)
        }
        if (url.protocol !== 'http:' || !isLoopback(url.hostname)) {
            throw new LocalFailure(`--local-graph is a zega-server on this machine (http://127.0.0.1:<port>); for ${url.origin} use --cloud-graph <id>`)
        }
        return { kind: 'local', url: url.origin, reachable: await answers(url.origin) }
    }
    return { kind: 'none' }
}
