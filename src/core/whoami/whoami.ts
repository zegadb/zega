import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'
import { withoutPrefix } from '@/utils/token_info'

/** Which token this is: name, scope, project restriction, expiry. */
export async function whoami(globals: Globals): Promise<void> {
    const { client, api } = await openSession(globals)
    await show(globals, withoutPrefix(await client.get('/v1/whoami')), (body) => render.whoami(body, api))
}
