import { load, readTokenFile } from '@/handlers/credentials'
import { normalizeApi } from '@/utils/api_address'
import { DASHBOARD, DEFAULT_API } from '@/utils/constants'
import { CloudClient } from '@/utils/cloud_request'
import { LocalFailure } from '@/utils/errors'

/** The options every command takes. */
export type Globals = {
    /** The API address; `login` stores it with the token. */
    api?: string
    /** Use the token in this file instead of the stored one: for CI and agents. */
    tokenFile?: string
    /** Readable output for people (tables, banners, questions on a terminal) instead of JSON. */
    human?: boolean
    /** Compact JSON, on a terminal too. */
    nopretty?: boolean
}

export type Session = { client: CloudClient; api: string }

/**
 * The token and host a command uses. `--token-file` brings its own token and
 * goes to `--api` or production; a stored token only ever goes to the host it
 * was made for.
 */
export async function openSession(globals: Globals): Promise<Session> {
    const requested = globals.api === undefined ? null : normalizeApi(globals.api)
    if (globals.tokenFile !== undefined) {
        const token = readTokenFile(globals.tokenFile)
        const api = requested ?? DEFAULT_API
        return { client: new CloudClient(api, token), api }
    }
    const stored = await load()
    if (stored === null) {
        throw new LocalFailure(
            `not logged in: run \`zega login\` (the token is made at ${DASHBOARD}/tokens), or pass --token-file`,
            'not_logged_in',
        )
    }
    const api = normalizeApi(stored.api)
    if (requested !== null && requested !== api) {
        throw new LocalFailure(
            `you are logged in to ${api}, not ${requested}: run \`zega login --api ${requested}\` first, or pass --token-file`,
        )
    }
    return { client: new CloudClient(api, stored.token), api }
}

/** Like `openSession`, but with no credentials at all it answers null (for a command that can do without). */
export async function openSessionIfAny(globals: Globals): Promise<Session | null> {
    if (globals.tokenFile === undefined && (await load()) === null) return null
    return await openSession(globals)
}
