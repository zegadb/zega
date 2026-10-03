import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** This month's usage and bill per graph. */
export async function usage(globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    await show(globals, await client.get('/v1/usage'), render.usage)
}
