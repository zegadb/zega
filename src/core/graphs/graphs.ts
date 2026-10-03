import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** List graphs, or show one. */
export async function graphs(id: string | undefined, globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    if (id === undefined) return await show(globals, await client.get('/v1/graphs'), render.graphs)
    await show(globals, await client.get('/v1/graphs/{graph}', { path: { graph: id } }), render.graphOne)
}
