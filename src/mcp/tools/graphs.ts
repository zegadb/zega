import { z } from 'zod'

import * as core from '@/core'
import { call, cloudUrl, type Target } from '@/handlers/graph_http'
import { commandGlobals, json, printed, type Context } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'
import { LocalFailure } from '@/utils/errors'

/** A graph's own address and the key that opens it. */
async function graphTarget(id: string, context: Context): Promise<Target> {
    const { client } = await context.session()
    const { graph } = (await client.get('/v1/graphs/{graph}', { path: { graph: id } })).json
    if (graph.hostUrl === null) throw new LocalFailure(`graph ${graph.id} has no address yet (status ${graph.status})`)
    return { url: cloudUrl(graph.hostUrl), key: await context.keys.graphKey(id) }
}

const ZQL_FIELDS = {
    query: z.string().min(1).describe('ZQL source. See zega_docs_search for the language.'),
    schema: z.string().optional().describe('The ZQL schema text. Leave out to use the schema stored on the graph (zega_graph_schema); not with document.'),
    document: z.boolean().optional().describe('true when `query` is a whole ZQL document that holds its own `schema { }` block, then `query { }` or `mutation { }` blocks.'),
}

/** The schema a query carries: the one given, none for a document, or the graph's stored one. */
async function schemaFor(target: Target, given: string | undefined, document: boolean | undefined): Promise<string> {
    if (document === true) return ''
    if (given !== undefined) return given
    const stored = (await call(target, 'GET', '/schema')) as { schema?: unknown } | null
    if (typeof stored?.schema !== 'string' || stored.schema === '') {
        throw new LocalFailure('this graph has no stored schema: pass `schema`, or `document: true` with a document that holds its own schema { } block', 'no_schema')
    }
    return stored.schema
}

export const graphs = [
    tool({
        ...read,
        name: 'zega_graphs',
        group: 'graphs',
        title: 'Graphs',
        description: 'List the account\'s graphs, or with an id show one: its project, region, size, status and address.',
        input: { id: z.string().optional().describe('A graph id (18 characters). Leave out to list every graph.') },
        run: async ({ id }, { globals }) => await printed(() => core.graphs(id, commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_graph_schema',
        group: 'graphs',
        title: 'A graph\'s schema',
        description: 'The ZQL schema text stored on a cloud graph (what the explorer and dashboard console open with); empty before the first push. Reading it counts as one query on the graph\'s meter. Needs a graph key: see zega_graph_query.',
        input: { graph: z.string().describe('A graph id.') },
        run: async ({ graph }, context) => json(await call(await graphTarget(graph, context), 'GET', '/schema')),
    }),
    tool({
        ...read,
        name: 'zega_graph_stats',
        group: 'graphs',
        title: 'A graph\'s usage',
        description: 'A graph\'s usage and size this month: queries, writes, storage, egress, the node count against what its plan includes, and the bill. From the API, so it needs no graph key.',
        input: { graph: z.string().describe('A graph id.') },
        run: async ({ graph }, { session }) => json((await (await session()).client.get('/v1/graphs/{graph}/usage', { path: { graph } })).json),
    }),
    tool({
        ...read,
        name: 'zega_graph_query',
        group: 'graphs',
        title: 'Run a ZQL query on a cloud graph',
        description:
            'Run a ZQL read on a cloud graph. It is sent with the HTTP QUERY method, which the graph itself refuses to run a mutation with, so this tool cannot change data. Counts as one query on the graph\'s meter. ' +
            'A graph is opened with a graph key (zk_...), not the API token: this server uses the one given with --graph-key-file or ZEGA_GRAPH_KEY in .dev.vars, else (a manage token, not --read-only) it makes a key for this session, named "zega mcp <time>", and revokes it when the server stops.',
        input: { graph: z.string().describe('A graph id.'), ...ZQL_FIELDS },
        run: async ({ graph, query, schema, document }, context) => {
            const target = await graphTarget(graph, context)
            return json(await call(target, 'QUERY', '/zql', { schema: await schemaFor(target, schema, document), query, document: document === true }))
        },
    }),
    tool({
        writes: true,
        manage: true,
        name: 'zega_graph_mutate',
        group: 'graphs',
        title: 'Run a ZQL mutation on a cloud graph',
        description:
            'Run ZQL that changes a cloud graph (a `mutation { }` block). It changes live data and cannot be undone: look first with zega_graph_query. Counts as a write on the graph\'s meter and is held by its spending cap. Needs a graph key, as zega_graph_query does.',
        input: { graph: z.string().describe('A graph id.'), ...ZQL_FIELDS },
        run: async ({ graph, query, schema, document }, context) => {
            const target = await graphTarget(graph, context)
            return json(await call(target, 'POST', '/zql', { schema: await schemaFor(target, schema, document), query, document: document === true }))
        },
    }),
]
