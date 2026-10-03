import path from 'node:path'

import { z } from 'zod'

import { call, localUrl } from '@/handlers/graph_http'
import { resolveServer } from '@/handlers/server_binary'
import { json } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'
import { LocalFailure } from '@/utils/errors'

const URL_FIELD = z.string().optional().describe('The server\'s address (http://127.0.0.1:<port>). Default: the one this session started last, else http://127.0.0.1:9342 (`zega start`\'s default).')

const ZQL_FIELDS = {
    query: z.string().min(1).describe('ZQL source. See zega_docs_search for the language.'),
    schema: z.string().optional().describe('The ZQL schema text. A local server keeps none: pass it, or use `document`.'),
    document: z.boolean().optional().describe('true when `query` is a whole ZQL document that holds its own `schema { }` block, then `query { }` or `mutation { }` blocks.'),
}

/** The data directory, inside the project. */
function dataDir(data: string | undefined, cwd: string): string {
    const root = path.resolve(cwd)
    const dir = path.resolve(root, data ?? 'zega-data')
    const relative = path.relative(root, dir)
    if (relative === '' || relative.startsWith('..') || path.isAbsolute(relative)) {
        throw new LocalFailure(`the data directory ${data} is outside the project (${cwd}); use one inside it`, 'outside_project')
    }
    return dir
}

export const local = [
    tool({
        writes: true,
        manage: false,
        name: 'zega_local_start',
        group: 'local',
        title: 'Start a local zega server',
        description:
            'Start the zega-server that ships with this CLI (what `zega start` runs) on a data directory inside the project, on a free port of 127.0.0.1 with no token, and answer its address. Creates the directory if it is missing. One server per directory; asking again returns the running one. It is stopped when this MCP server stops.',
        input: { data: z.string().optional().describe('Data directory, relative to the project (default zega-data).') },
        run: async ({ data }, { cwd, local, serverPath }) => json(await local.start(resolveServer({ override: serverPath }), dataDir(data, cwd))),
    }),
    tool({
        writes: true,
        manage: false,
        name: 'zega_local_stop',
        group: 'local',
        title: 'Stop a local zega server',
        description: 'Stop a local server this session started (cleanly: the database is shut down, nothing is lost). A server somebody else started is not touched.',
        input: { data: z.string().optional().describe('Its data directory. Leave out when only one is running.') },
        run: async ({ data }, { cwd, local }) => json(await local.stop(data === undefined ? undefined : dataDir(data, cwd))),
    }),
    tool({
        ...read,
        name: 'zega_local_status',
        group: 'local',
        title: 'Is a local zega server running',
        description: 'The local servers this session started, and whether the server at an address answers, with its node and relationship counts.',
        input: { url: URL_FIELD },
        run: async ({ url }, { local }) => {
            const address = localUrl(url ?? local.defaultUrl())
            let stats: unknown = null
            let running = true
            try {
                stats = await call({ url: address }, 'GET', '/stats')
            } catch (error) {
                // A server that answers but refuses (a token, an older build without /stats) is running; one that does not answer is not.
                running = error instanceof LocalFailure
            }
            return json({ url: address, running, stats, startedHere: local.list() })
        },
    }),
    tool({
        ...read,
        name: 'zega_local_query',
        group: 'local',
        title: 'Run a ZQL query on a local server',
        description: 'Run a ZQL read on a local zega server. It is sent with the HTTP QUERY method, which the server itself refuses to run a mutation with, so this tool cannot change data.',
        input: { url: URL_FIELD, ...ZQL_FIELDS },
        run: async ({ url, query, schema, document }, { local }) =>
            json(await call({ url: localUrl(url ?? local.defaultUrl()) }, 'QUERY', '/zql', { schema: schema ?? '', query, document: document === true })),
    }),
    tool({
        writes: true,
        manage: false,
        name: 'zega_local_mutate',
        group: 'local',
        title: 'Run a ZQL mutation on a local server',
        description: 'Run ZQL that changes a local zega server\'s database (a `mutation { }` block). It writes to the data directory and cannot be undone.',
        input: { url: URL_FIELD, ...ZQL_FIELDS },
        run: async ({ url, query, schema, document }, { local }) =>
            json(await call({ url: localUrl(url ?? local.defaultUrl()) }, 'POST', '/zql', { schema: schema ?? '', query, document: document === true })),
    }),
]
