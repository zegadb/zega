import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** List functions, or show one (its variables and secret names too). */
export async function functions(id: string | undefined, globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    if (id === undefined) return await show(globals, await client.get('/v1/functions'), render.functions)
    await show(globals, await client.get('/v1/functions/{function}', { path: { function: id } }), render.functionOne)
}
