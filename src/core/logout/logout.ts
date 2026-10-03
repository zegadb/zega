import { credentials, type Globals } from '@/handlers'
import { DASHBOARD } from '@/utils/constants'
import { json, write } from '@/utils'

/** Remove the stored token. The token itself stays valid until it is revoked in the dashboard. */
export async function logout(globals: Globals): Promise<void> {
    const removed = await credentials.remove()
    if (globals.human !== true) {
        await write('stdout', json({ loggedOut: removed !== null }))
        return
    }
    await write(
        'stdout',
        removed === null
            ? 'not logged in: no stored token\n'
            : `removed ${removed}\nThe token itself is still valid until you revoke it at ${DASHBOARD}/tokens\n`,
    )
}
