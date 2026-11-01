import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** List projects, or show one with its graphs. */
export async function projects(id: string | undefined, globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    if (id === undefined) return await show(globals, await client.get('/v1/projects'), render.projects)
    await show(globals, await client.get('/v1/projects/{project}', { path: { project: id } }), render.projectOne)
}
