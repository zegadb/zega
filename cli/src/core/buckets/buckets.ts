import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

/** List storage buckets, or show one. */
export async function buckets(id: string | undefined, globals: Globals): Promise<void> {
    const { client } = await openSession(globals)
    if (id === undefined) return await show(globals, await client.get('/v1/buckets'), render.buckets)
    await show(globals, await client.get('/v1/buckets/{bucket}', { path: { bucket: id } }), render.bucketOne)
}
