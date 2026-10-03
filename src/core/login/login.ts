import { DEFAULT_API } from '@/utils/constants'
import { credentials } from '@/handlers'
import type { Store } from '@/handlers/credentials'
import type { Globals } from '@/handlers'
import * as questions from '@/questions'
import { CloudClient } from '@/utils/cloud_request'
import { normalizeApi } from '@/utils/api_address'
import { LocalFailure } from '@/utils/errors'
import * as render from '@/utils/render'
import { answer, canPrompt, show, welcome, write } from '@/utils'

export type LoginOptions = Globals & { store?: string }

/**
 * Store an API token. The token comes from `--token-file`, from stdin when it
 * is piped, or from a hidden prompt on a terminal; never from an argument
 * (arguments show in process listings). It is checked with the API before it is
 * saved: a mistyped token never replaces a good one.
 */
export async function login(options: LoginOptions): Promise<void> {
    const api = normalizeApi(options.api ?? DEFAULT_API)
    const store = parseStore(options.store)
    const prompt = canPrompt(options)

    let given: string | undefined
    if (options.tokenFile !== undefined) {
        given = credentials.readTokenFile(options.tokenFile)
    } else if (process.stdin.isTTY !== true) {
        const piped = await credentials.readStdin()
        if (piped.trim() !== '') {
            given = credentials.oneToken(piped) ?? undefined
            if (given === undefined) {
                throw new LocalFailure('the token on stdin must be one nonempty word, with no spaces or line breaks')
            }
        }
    }
    if (prompt && given === undefined) await welcome('login', 'store a Zega Cloud API token')
    const token = await answer({
        flag: '--token-file',
        given,
        hint: 'a token (made at the dashboard) piped on stdin or read from a file',
        prompt,
        validate: questions.validateToken,
        ask: questions.token,
    })

    const reply = await new CloudClient(api, token).get('/v1/whoami')
    await credentials.save(api, token, store)
    if (options.json === true) return await show(options, reply, () => '')
    await write('stdout', render.login(reply.json))
}

function parseStore(value: string | undefined): Store {
    if (value === undefined || value === 'file' || value === 'keychain') return value ?? 'file'
    throw new LocalFailure(`--store ${JSON.stringify(value)}: it is \`file\` (the default) or \`keychain\``)
}
