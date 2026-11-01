import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** The regions a graph can run in. */
export async function regions(globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    await show(globals, await client.get('/v1/regions'), render.regions)
}
